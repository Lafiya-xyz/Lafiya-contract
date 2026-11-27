//! Soroban custom account contract implementing weighted, role-based multisig authorization.
//!
//! This contract enforces minimum signer, voting-weight, and role quorums.
//! It implements Soroban's `CustomAccountInterface` to integrate with the
//! protocol's authentication system, enabling use as a custom account for
//! administrative operations on the `attester-registry` and
//! `attestation-registry` contracts.
#![no_std]
#![cfg_attr(
    dylint_lib = "lafiya_lints",
    allow(
        storage_in_loop,
        persistent_set_without_ttl,
        unchecked_amount_arithmetic,
        reason = "baseline: pre-existing findings, tracked for per-site cleanup (docs/custom-lints.md)"
    )
)]

// Unlike `attester-registry` and `attestation-registry`, this crate does not set
// `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. `__constructor`
// cannot return a `Result` (Soroban constructors return `()`), so it deliberately uses
// `panic_with_error!` to reject invalid construction parameters with a contract error
// code, which the blanket deny would flag despite being the correct pattern here.

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    panic_with_error, Address, Bytes, BytesN, Env, Symbol, Vec,
};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The minimum number of signatures required to authorize a transaction.
    Threshold,
    /// Presence of this key (mapped to `()`) indicates the public key is a registered signer.
    Signer(BytesN<32>),
    /// The total number of registered signers.
    SignerCount,
    /// The ordered list of registered signers, used to safely replace the set.
    SignerSet,
    /// Per-signer weight and role used during authorization.
    SignerDetails(BytesN<32>),
    /// The current weighted and role-based quorum policy.
    Policy,
    /// Membership marker for Ed25519 and secp256r1 signers.
    TypedSigner(SignerKey),
    /// Weighted and role configuration for a typed signer key.
    TypedSignerDetails(SignerKey),
    /// Ordered typed signer keys used when replacing the set.
    TypedSignerSet,
    /// Policy override for a specific `(contract_address, fn_name)` context.
    ContextPolicy(Address, Symbol),
    /// Ordered list of configured context-policy keys, used to enumerate overrides.
    ContextPolicyKeys,
    /// Default policy applied to contexts without an override.
    DefaultPolicy,
    /// Monotonic configuration version, bumped on every policy mutation.
    ConfigVersion,
}

/// Public key type used by an authorized signer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignerKey {
    /// Ed25519 public key.
    Ed25519(BytesN<32>),
    /// Uncompressed SEC-1 secp256r1 public key.
    Secp256r1(BytesN<65>),
}

impl SignerKey {
    fn is_before(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ed25519(left), Self::Ed25519(right)) => left.to_array() < right.to_array(),
            (Self::Ed25519(_), Self::Secp256r1(_)) => true,
            (Self::Secp256r1(_), Self::Ed25519(_)) => false,
            (Self::Secp256r1(left), Self::Secp256r1(right)) => left.to_array() < right.to_array(),
        }
    }
}

/// A configured signer and the voting weight/role assigned to that signer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerConfig {
    /// Ed25519 or uncompressed SEC-1 secp256r1 public key.
    pub public_key: SignerKey,
    /// Voting weight contributed toward the weighted quorum.
    pub weight: u32,
    /// Optional role used to satisfy one or more role minimums.
    pub role: Option<soroban_sdk::Symbol>,
}

/// A minimum number of approvals required from one role.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRequirement {
    /// The role required in the approving signer set.
    pub role: soroban_sdk::Symbol,
    /// Minimum number of approving signers that must have this role.
    pub minimum: u32,
}

/// Full multisig policy returned by `get_policy`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerPolicy {
    /// Configured Ed25519 and secp256r1 signers.
    pub signers: Vec<SignerConfig>,
    /// Minimum sum of signer weights among approvals.
    pub weight_threshold: u32,
    /// Minimum number of distinct signer approvals.
    pub minimum_signers: u32,
    /// Additional role-specific approval minimums.
    pub role_requirements: Vec<RoleRequirement>,
}

/// A single ed25519 signature from one signer in the multisig set.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signature {
    /// The public key of the signer who created this signature.
    pub public_key: SignerKey,
    /// The 64-byte signature; P-256 uses low-S r || s over the authorization hash.
    pub signature: BytesN<64>,
}

/// Errors returned by the multisig-account contract's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The configured threshold is zero or exceeds the signer count.
    InvalidThreshold = 1,
    /// The signer configuration contains duplicate public keys.
    DuplicateSigner = 2,
    /// The supplied signature count is below the configured threshold.
    NotEnoughSigners = 3,
    /// Signatures are not strictly ordered by key type and public-key bytes.
    BadSignatureOrder = 4,
    /// A signature corresponds to a public key that is not a configured signer.
    UnknownSigner = 5,
    /// The contract has not been initialized; threshold or signer count is unavailable.
    NotInitialized = 6,
    /// The supplied signature count exceeds the configured signer count.
    TooManySigners = 7,
    /// The supplied signatures do not meet the configured weight threshold.
    NotEnoughWeight = 8,
    /// The supplied signatures do not meet one or more role requirements.
    RoleQuorumNotMet = 9,
    /// A signer weight, role requirement, or policy threshold is invalid.
    InvalidPolicy = 10,
    /// The supplied signature count exceeds `MAX_SIGNERS`.
    SignerLimitExceeded = 11,
    /// The supplied client data exceeds `MAX_CLIENT_DATA_LEN`.
    ClientDataTooLong = 12,
    /// The replacement signer key is already present in the policy.
    SignerAlreadyPresent = 13,
    /// The signer to replace is not present in the policy.
    SignerNotFound = 14,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Interface kind reported by `get_interface`, used by clients as a weak
/// identity check when wiring contracts (not proof of authenticity).
pub const CONTRACT_KIND: &str = "lafiya_multisig_account";

/// Version of the public contract interface (functions, errors, types).
/// Bump on any breaking ABI change; `scripts/conformance/check_snapshot.py`
/// refuses a breaking snapshot update without a bump.
pub const INTERFACE_VERSION: u32 = 1;

// Source-provenance metadata (SEP-46 `contractmetav0`); policy in
// docs/stability-policy.md, enforced by scripts/check_wasm_meta.py.
soroban_sdk::contractmeta!(
    key = "source_repo",
    val = "github:Lafiya-xyz/Lafiya-contract"
);
soroban_sdk::contractmeta!(key = "home_domain", val = "lafiya-xyz.github.io");
soroban_sdk::contractmeta!(key = "crate_name", val = env!("CARGO_PKG_NAME"));
soroban_sdk::contractmeta!(key = "crate_version", val = env!("CARGO_PKG_VERSION"));
soroban_sdk::contractmeta!(key = "source_rev", val = env!("LAFIYA_GIT_COMMIT"));
soroban_sdk::contractmeta!(key = "interface_version", val = "1");

/// Version of the emitted event schemas (see `docs/events.md`).
pub const EVENT_VERSION: u32 = 1;

/// Maximum number of signatures accepted by `__check_auth`.
///
/// Derived from the worst-case mixed-key CPU budget measurements published in
/// `docs/benchmarks.md`: an authorization with `MAX_SIGNERS` secp256r1
/// (passkey) signatures and maximum client data must consume at most 50% of
/// the transaction CPU limit in `config/network-limits.json`. The worst case
/// is dominated by secp256r1 verification, so the bound is chosen against an
/// all-passkey set. Re-derive with `lafiya-bench` before changing.
pub const MAX_SIGNERS: u32 = 16;

/// Maximum length, in bytes, of WebAuthn client data accepted per signature.
///
/// Bounds the SHA-256 hashing cost included in the worst-case budget above.
pub const MAX_CLIENT_DATA_LEN: u32 = 1024;

/// Optional features this build supports, reported by `get_interface`.
pub const FEATURES: [&str; 2] = ["ed25519", "unscoped_auth"];

/// Interface and capability metadata returned by `get_interface`, so
/// clients can negotiate features with one call instead of probing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceInfo {
    /// Contract kind, e.g. `lafiya_multisig_account`.
    pub contract_kind: Symbol,
    /// Public interface version; bumped on any breaking ABI change.
    pub interface_version: u32,
    /// Optional features enabled in this build.
    pub features: Vec<Symbol>,
    /// Storage schema version (this contract has no versioned storage, so always 1).
    pub schema_version: u32,
    /// Event schema version.
    pub event_version: u32,
}

/// Storage schema version; this contract's storage is not versioned.
const SCHEMA_VERSION: u32 = 1;

/// Emitted when the signer set is configured or replaced.
#[contractevent]
#[derive(Clone, Debug)]
pub struct MultisigConfigured {
    #[topic]
    pub threshold: u32,
    /// The configured signer keys in canonical order.
    pub signers: Vec<SignerKey>,
}

/// Emitted when `__check_auth` accepts an authorization.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AuthorizationApproved {
    #[topic]
    pub timestamp: u64,
    /// The signature payload that was authorized.
    pub payload: Bytes,
    /// The signer keys whose signatures were verified, in signature order.
    pub signers: Vec<SignerKey>,
}

/// Emitted when a single signer is atomically replaced via `replace_signer`.
#[contractevent]
#[derive(Clone, Debug)]
pub struct SignerReplaced {
    #[topic]
    pub old_fingerprint: BytesN<32>,
    /// Fingerprint of the replacement signer key.
    pub new_fingerprint: BytesN<32>,
    /// Configuration version after the replacement.
    pub config_version: u32,
}

/// Emitted when a context policy override is configured or removed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ContextPolicySet {
    #[topic]
    pub contract: Address,
    /// The function name the override applies to.
    pub function: Symbol,
    /// Whether the override was removed (falling back to the default policy).
    pub removed: bool,
    /// Configuration version after the change.
    pub config_version: u32,
}

#[contract]
pub struct MultisigAccount;

#[contractimpl]
impl MultisigAccount {
    /// Initialize the multisig account with a set of authorized signers and a signature threshold.
    ///
    /// # Arguments
    /// * `signers` — A vector of ed25519 public keys (32 bytes each) authorized to sign transactions.
    /// * `threshold` — The minimum number of signatures required to authorize a transaction; must be > 0 and ≤ the signer count.
    pub fn __constructor(env: Env, signers: Vec<BytesN<32>>, threshold: u32) {
        if threshold == 0 || threshold > signers.len() {
            panic_with_error!(&env, Error::InvalidThreshold);
        }

        let mut signer_configs = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in signers.iter() {
            let key = DataKey::Signer(signer.clone());
            if env.storage().instance().has(&key) {
                panic_with_error!(&env, Error::DuplicateSigner);
            }
            env.storage().instance().set(&key, &());
            let public_key = SignerKey::Ed25519(signer.clone());
            let config = SignerConfig {
                public_key: public_key.clone(),
                weight: 1,
                role: None,
            };
            env.storage()
                .instance()
                .set(&DataKey::TypedSigner(public_key.clone()), &());
            env.storage()
                .instance()
                .set(&DataKey::TypedSignerDetails(public_key.clone()), &config);
            signer_configs.push_back(config);
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &signers.len());
        env.storage().instance().set(&DataKey::SignerSet, &signers);
        env.storage().instance().set(
            &DataKey::Policy,
            &SignerPolicy {
                signers: signer_configs,
                weight_threshold: threshold,
                minimum_signers: threshold,
                role_requirements: Vec::new(&env),
            },
        );
        let mut typed_signers = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in signers.iter() {
            typed_signers.push_back(SignerKey::Ed25519(signer));
        }
        env.storage()
            .instance()
            .set(&DataKey::TypedSignerSet, &typed_signers);
        MultisigConfigured {
            threshold,
            signers: Self::sorted_signers(&env, &typed_signers),
        }
        .publish(&env);
    }

    /// Return the current signer set in the canonical order required by
    /// `__check_auth` (key type, then ascending key bytes).
    pub fn get_signers(env: Env) -> Result<Vec<SignerKey>, Error> {
        if let Some(signers) = env
            .storage()
            .instance()
            .get::<_, Vec<SignerKey>>(&DataKey::TypedSignerSet)
        {
            return Ok(Self::sorted_signers(&env, &signers));
        }

        let legacy_signers: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerSet)
            .ok_or(Error::NotInitialized)?;
        let mut signers = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in legacy_signers.iter() {
            signers.push_back(SignerKey::Ed25519(signer));
        }
        Ok(Self::sorted_signers(&env, &signers))
    }

    fn sorted_signers(env: &Env, signers: &Vec<SignerKey>) -> Vec<SignerKey> {
        let mut sorted: Vec<SignerKey> = Vec::new(env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in signers.iter() {
            let mut next = Vec::new(env);
            let mut inserted = false;
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
            for current in sorted.iter() {
                if !inserted && signer.is_before(&current) {
                    next.push_back(signer.clone());
                    inserted = true;
                }
                next.push_back(current);
            }
            if !inserted {
                next.push_back(signer);
            }
            sorted = next;
        }
        sorted
    }

    /// Return the minimum number of signer approvals required.
    pub fn get_threshold(env: Env) -> Result<u32, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)
    }

    /// Return the current weighted and role-based signer policy.
    pub fn get_policy(env: Env) -> Result<SignerPolicy, Error> {
        Self::policy(&env)
    }

    /// Return the default policy applied to contexts without an override.
    pub fn get_default_policy(env: Env) -> Result<SignerPolicy, Error> {
        Self::default_policy(&env)
    }

    /// Return the policy override for a `(contract, function)` context, if any.
    pub fn get_context_policy(
        env: Env,
        contract: Address,
        function: Symbol,
    ) -> Option<SignerPolicy> {
        env.storage()
            .instance()
            .get(&DataKey::ContextPolicy(contract, function))
    }

    /// Return the current configuration version.
    pub fn get_config_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ConfigVersion)
            .unwrap_or(0)
    }

    /// Configure a policy override for a `(contract, function)` context.
    ///
    /// The change itself must be authorized under the strictest configured
    /// policy, so a routine context cannot be used to weaken the account.
    pub fn set_context_policy(
        env: Env,
        contract: Address,
        function: Symbol,
        policy: SignerPolicy,
    ) -> Result<(), Error> {
        env.current_contract_address().require_auth();
        Self::validate_policy(
            &policy.signers,
            policy.weight_threshold,
            policy.minimum_signers,
            &policy.role_requirements,
        )?;

        let key = DataKey::ContextPolicy(contract.clone(), function.clone());
        if !env.storage().instance().has(&key) {
            let mut keys: Vec<(Address, Symbol)> = env
                .storage()
                .instance()
                .get(&DataKey::ContextPolicyKeys)
                .unwrap_or_else(|| Vec::new(&env));
            keys.push_back((contract.clone(), function.clone()));
            env.storage()
                .instance()
                .set(&DataKey::ContextPolicyKeys, &keys);
        }
        env.storage().instance().set(&key, &policy);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        let config_version = Self::bump_config_version(&env);
        ContextPolicySet {
            contract,
            function,
            removed: false,
            config_version,
        }
        .publish(&env);
        Ok(())
    }

    /// Remove a policy override, falling back to the default policy.
    pub fn remove_context_policy(
        env: Env,
        contract: Address,
        function: Symbol,
    ) -> Result<(), Error> {
        env.current_contract_address().require_auth();
        let key = DataKey::ContextPolicy(contract.clone(), function.clone());
        env.storage().instance().remove(&key);

        let keys: Vec<(Address, Symbol)> = env
            .storage()
            .instance()
            .get(&DataKey::ContextPolicyKeys)
            .unwrap_or_else(|| Vec::new(&env));
        let mut remaining = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured context-policy key list
        for entry in keys.iter() {
            if entry.0 != contract || entry.1 != function {
                remaining.push_back(entry);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::ContextPolicyKeys, &remaining);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        let config_version = Self::bump_config_version(&env);
        ContextPolicySet {
            contract,
            function,
            removed: true,
            config_version,
        }
        .publish(&env);
        Ok(())
    }

    /// Atomically replace one signer with another, preserving weight and role
    /// unless `new` explicitly overrides them.
    ///
    /// Rejects a replacement key that is already present and runs the same
    /// satisfiability checks as `set_policy`. The account must authorize this
    /// call under its current policy.
    pub fn replace_signer(
        env: Env,
        old: SignerKey,
        new: SignerConfig,
    ) -> Result<(), Error> {
        env.current_contract_address().require_auth();

        let mut policy = Self::policy(&env)?;
        let mut found: Option<u32> = None;
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set
        for (index, signer) in policy.signers.iter().enumerate() {
            if signer.public_key == old {
                found = Some(index);
            } else if signer.public_key == new.public_key {
                return Err(Error::SignerAlreadyPresent);
            }
        }
        let index = found.ok_or(Error::SignerNotFound)?;

        let previous = policy.signers.get_unchecked(index);
        let replacement = SignerConfig {
            public_key: new.public_key.clone(),
            weight: if new.weight == 0 {
                previous.weight
            } else {
                new.weight
            },
            role: match &new.role {
                Some(role) => Some(role.clone()),
                None => previous.role.clone(),
            },
        };

        let mut signers = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set
        for (i, signer) in policy.signers.iter().enumerate() {
            if i == index {
                signers.push_back(replacement.clone());
            } else {
                signers.push_back(signer);
            }
        }

        Self::validate_policy(
            &signers,
            policy.weight_threshold,
            policy.minimum_signers,
            &policy.role_requirements,
        )?;

        // Rewrite the typed-signer indexes for the replaced key.
        env.storage()
            .instance()
            .remove(&DataKey::TypedSigner(old.clone()));
        env.storage()
            .instance()
            .remove(&DataKey::TypedSignerDetails(old.clone()));
        if let SignerKey::Ed25519(public_key) = &old {
            env.storage()
                .instance()
                .remove(&DataKey::Signer(public_key.clone()));
            env.storage()
                .instance()
                .remove(&DataKey::SignerDetails(public_key.clone()));
        }
        env.storage()
            .instance()
            .set(&DataKey::TypedSigner(replacement.public_key.clone()), &());
        env.storage().instance().set(
            &DataKey::TypedSignerDetails(replacement.public_key.clone()),
            &replacement,
        );
        if let SignerKey::Ed25519(public_key) = &replacement.public_key {
            env.storage()
                .instance()
                .set(&DataKey::Signer(public_key.clone()), &());
        }

        let mut typed_signers = Vec::new(&env);
        let mut legacy_signers = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set
        for signer in signers.iter() {
            typed_signers.push_back(signer.public_key.clone());
            if let SignerKey::Ed25519(public_key) = &signer.public_key {
                legacy_signers.push_back(public_key.clone());
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::TypedSignerSet, &typed_signers);
        env.storage()
            .instance()
            .set(&DataKey::SignerSet, &legacy_signers);

        policy.signers = signers;
        env.storage().instance().set(&DataKey::Policy, &policy);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        let config_version = Self::bump_config_version(&env);
        SignerReplaced {
            old_fingerprint: Self::fingerprint(&env, &old),
            new_fingerprint: Self::fingerprint(&env, &replacement.public_key),
            config_version,
        }
        .publish(&env);
        Ok(())
    }

    fn fingerprint(env: &Env, key: &SignerKey) -> BytesN<32> {
        let bytes: Bytes = match key {
            SignerKey::Ed25519(public_key) => public_key.clone().into(),
            SignerKey::Secp256r1(public_key) => public_key.clone().into(),
        };
        env.crypto().sha256(&bytes).to_bytes()
    }

    fn bump_config_version(env: &Env) -> u32 {
        let next = env
            .storage()
            .instance()
            .get::<_, u32>(&DataKey::ConfigVersion)
            .unwrap_or(0)
            .saturating_add(1);
        env.storage().instance().set(&DataKey::ConfigVersion, &next);
        next
    }

    fn default_policy(env: &Env) -> Result<SignerPolicy, Error> {
        if let Some(policy) = env.storage().instance().get(&DataKey::DefaultPolicy) {
            return Ok(policy);
        }
        Self::policy(env)
    }

    /// Resolve the policy for a single auth context.
    ///
    /// Contract-creation contexts and any context without an override fall
    /// back to the strictest configured policy.
    fn context_policy(env: &Env, context: &Context) -> Result<SignerPolicy, Error> {
        match context {
            Context::Contract(contract) => {
                if let Some(policy) = env.storage().instance().get(&DataKey::ContextPolicy(
                    contract.contract.clone(),
                    contract.fn_name.clone(),
                )) {
                    Ok(policy)
                } else {
                    Self::strictest_policy(env)
                }
            }
            Context::CreateContractHostFn(_) | Context::CreateContractWithCtorHostFn(_) => {
                Self::strictest_policy(env)
            }
        }
    }

    /// Compute the strictest policy across the default policy and every
    /// configured context override.
    fn strictest_policy(env: &Env) -> Result<SignerPolicy, Error> {
        let mut strictest = Self::default_policy(env)?;
        let keys: Vec<(Address, Symbol)> = env
            .storage()
            .instance()
            .get(&DataKey::ContextPolicyKeys)
            .unwrap_or_else(|| Vec::new(env));
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured context-policy key list
        for (contract, function) in keys.iter() {
            if let Some(policy) = env
                .storage()
                .instance()
                .get::<_, SignerPolicy>(&DataKey::ContextPolicy(contract, function))
            {
                if Self::is_stricter(&policy, &strictest) {
                    strictest = policy;
                }
            }
        }
        Ok(strictest)
    }

    /// Return `true` when `candidate` requires at least as much as `current`
    /// on every dimension (weight, signer count, and each role minimum).
    fn is_stricter(candidate: &SignerPolicy, current: &SignerPolicy) -> bool {
        if candidate.weight_threshold < current.weight_threshold
            || candidate.minimum_signers < current.minimum_signers
        {
            return false;
        }
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured role requirements
        for requirement in current.role_requirements.iter() {
            let mut candidate_minimum = 0u32;
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured role requirements
            for other in candidate.role_requirements.iter() {
                if other.role == requirement.role {
                    candidate_minimum = other.minimum;
                }
            }
            if candidate_minimum < requirement.minimum {
                return false;
            }
        }
        true
    }

    /// Replace the signer set with equal-weight signers and no role constraints.
    ///
    /// The account itself must authorize this call under its current policy.
    pub fn set_signers(env: Env, signers: Vec<BytesN<32>>, threshold: u32) -> Result<(), Error> {
        let mut configs = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for public_key in signers.iter() {
            configs.push_back(SignerConfig {
                public_key: SignerKey::Ed25519(public_key),
                weight: 1,
                role: None,
            });
        }
        let role_requirements = Vec::new(&env);
        Self::set_policy(env, configs, threshold, threshold, role_requirements)
    }

    /// Atomically replace the signer set and approval policy.
    ///
    /// `minimum_signers` and `weight_threshold` must both be met. Each role
    /// requirement also specifies a minimum number of approving signers with
    /// that role. The current policy must authorize this change.
    pub fn set_policy(
        env: Env,
        signers: Vec<SignerConfig>,
        weight_threshold: u32,
        minimum_signers: u32,
        role_requirements: Vec<RoleRequirement>,
    ) -> Result<(), Error> {
        env.current_contract_address().require_auth();
        Self::validate_policy(
            &signers,
            weight_threshold,
            minimum_signers,
            &role_requirements,
        )?;

        let old_signers = match env
            .storage()
            .instance()
            .get::<_, Vec<SignerKey>>(&DataKey::TypedSignerSet)
        {
            Some(signers) => signers,
            None => {
                let legacy_signers: Vec<BytesN<32>> = env
                    .storage()
                    .instance()
                    .get(&DataKey::SignerSet)
                    .ok_or(Error::NotInitialized)?;
                let mut signers = Vec::new(&env);
                // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
                for signer in legacy_signers.iter() {
                    signers.push_back(SignerKey::Ed25519(signer));
                }
                signers
            }
        };

        let mut legacy_signers = Vec::new(&env);
        let mut typed_signers = Vec::new(&env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in old_signers.iter() {
            env.storage()
                .instance()
                .remove(&DataKey::TypedSigner(signer.clone()));
            env.storage()
                .instance()
                .remove(&DataKey::TypedSignerDetails(signer.clone()));
            if let SignerKey::Ed25519(public_key) = signer {
                env.storage()
                    .instance()
                    .remove(&DataKey::Signer(public_key.clone()));
                env.storage()
                    .instance()
                    .remove(&DataKey::SignerDetails(public_key.clone()));
            }
        }
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for signer in signers.iter() {
            env.storage()
                .instance()
                .set(&DataKey::TypedSigner(signer.public_key.clone()), &());
            env.storage().instance().set(
                &DataKey::TypedSignerDetails(signer.public_key.clone()),
                &signer,
            );
            if let SignerKey::Ed25519(public_key) = &signer.public_key {
                env.storage()
                    .instance()
                    .set(&DataKey::Signer(public_key.clone()), &());
                legacy_signers.push_back(public_key.clone());
            }
            typed_signers.push_back(signer.public_key.clone());
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &minimum_signers);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &signers.len());
        env.storage()
            .instance()
            .set(&DataKey::SignerSet, &legacy_signers);
        env.storage()
            .instance()
            .set(&DataKey::TypedSignerSet, &typed_signers);
        env.storage().instance().set(
            &DataKey::Policy,
            &SignerPolicy {
                signers,
                weight_threshold,
                minimum_signers,
                role_requirements,
            },
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        MultisigConfigured {
            threshold: minimum_signers,
            signers: Self::sorted_signers(&env, &typed_signers),
        }
        .publish(&env);

        Ok(())
    }

    fn validate_policy(
        signers: &Vec<SignerConfig>,
        weight_threshold: u32,
        minimum_signers: u32,
        role_requirements: &Vec<RoleRequirement>,
    ) -> Result<(), Error> {
        if minimum_signers == 0 || minimum_signers > signers.len() {
            return Err(Error::InvalidThreshold);
        }

        let mut total_weight = 0u32;
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for (index, signer) in signers.iter().enumerate() {
            if signer.weight == 0 {
                return Err(Error::InvalidPolicy);
            }
            if let SignerKey::Secp256r1(public_key) = &signer.public_key {
                if public_key.to_array()[0] != 4 {
                    return Err(Error::InvalidPolicy);
                }
            }
            total_weight = total_weight
                .checked_add(signer.weight)
                .ok_or(Error::InvalidPolicy)?;
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
            for previous in signers.iter().take(index) {
                if previous.public_key == signer.public_key {
                    return Err(Error::DuplicateSigner);
                }
            }
        }

        if weight_threshold == 0 || weight_threshold > total_weight {
            return Err(Error::InvalidPolicy);
        }

        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for (index, requirement) in role_requirements.iter().enumerate() {
            if requirement.minimum == 0 {
                return Err(Error::InvalidPolicy);
            }
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
            if role_requirements
                .iter()
                .take(index)
                .any(|previous| previous.role == requirement.role)
            {
                return Err(Error::InvalidPolicy);
            }
            let mut matching_signers = 0u32;
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
            for signer in signers.iter() {
                if signer.role == Some(requirement.role.clone()) {
                    matching_signers += 1;
                }
            }
            if requirement.minimum > matching_signers {
                return Err(Error::InvalidPolicy);
            }
        }
        Ok(())
    }

    fn policy(env: &Env) -> Result<SignerPolicy, Error> {
        if let Some(policy) = env.storage().instance().get(&DataKey::Policy) {
            return Ok(policy);
        }

        let minimum_signers: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)?;
        let legacy_signers: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerSet)
            .ok_or(Error::NotInitialized)?;
        let mut signers = Vec::new(env);
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for public_key in legacy_signers.iter() {
            signers.push_back(SignerConfig {
                public_key: SignerKey::Ed25519(public_key),
                weight: 1,
                role: None,
            });
        }
        Ok(SignerPolicy {
            signers,
            weight_threshold: minimum_signers,
            minimum_signers,
            role_requirements: Vec::new(env),
        })
    }

    /// Report the contract kind, interface version, enabled features, and
    /// storage/event schema versions for runtime compatibility negotiation.
    /// Callable by anyone.
    pub fn get_interface(env: Env) -> InterfaceInfo {
        let mut features = Vec::new(&env);
        // loop-bound: FEATURES -- fixed compile-time array
        for feature in FEATURES {
            features.push_back(Symbol::new(&env, feature));
        }
        InterfaceInfo {
            contract_kind: Symbol::new(&env, CONTRACT_KIND),
            interface_version: INTERFACE_VERSION,
            features,
            schema_version: SCHEMA_VERSION,
            event_version: EVENT_VERSION,
        }
    }
}

#[contractimpl(contracttrait)]
impl CustomAccountInterface for MultisigAccount {
    type Signature = Vec<Signature>;
    type Error = Error;

    /// Verify the authorization of a transaction using the configured quorum policy.
    ///
    /// Signatures must meet the minimum signer count and weight threshold, satisfy every role
    /// minimum, and be ordered by key type and ascending key bytes.
    ///
    /// # Arguments
    /// * `signature_payload` — A 32-byte hash of the transaction to authorize.
    /// * `signatures` — A vector of ed25519 signatures, each with a public key and signature bytes, ordered by ascending public key.
    /// * `auth_contexts` — The authorization contexts; the strictest policy
    ///   across all of them is required. Unknown and contract-creation
    ///   contexts fall back to the strictest configured policy. See
    ///   [ADR-0007](../../../docs/adr/0007-unscoped-multisig-authorization.md).
    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signatures: Self::Signature,
        auth_contexts: Vec<Context>,
    ) -> Result<(), Error> {
        let mut policy = Self::default_policy(&env)?;
        // loop-bound: ACCEPTED(admin) -- iterates the transaction's auth contexts
        for context in auth_contexts.iter() {
            let candidate = Self::context_policy(&env, &context)?;
            if Self::is_stricter(&candidate, &policy) {
                policy = candidate;
            }
        }

        if signatures.len() > MAX_SIGNERS {
            return Err(Error::SignerLimitExceeded);
        }

        if signatures.len() < policy.minimum_signers {
            return Err(Error::NotEnoughSigners);
        }

        let signer_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCount)
            .ok_or(Error::NotInitialized)?;

        if signatures.len() > signer_count {
            return Err(Error::TooManySigners);
        }

        // Enforce the client-data bound used to derive `MAX_SIGNERS`.
        // loop-bound: ACCEPTED(admin) -- iterates the supplied signatures
        for signature in signatures.iter() {
            if signature.signature.len() > MAX_CLIENT_DATA_LEN {
                return Err(Error::ClientDataTooLong);
            }
        }

        // Pass 1: cheap checks only (strict ordering doubles as duplicate
        // detection in O(n)), so malformed or under-quorum input exits before
        // any signature verification.
        let mut signer_configs = Vec::new(&env);
        let mut authorized_signers = Vec::new(&env);
        let mut signed_weight = 0u32;
        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for index in 0..signatures.len() {
            let signature = signatures.get_unchecked(index);
            if index > 0 {
                let previous = signatures.get_unchecked(index - 1);
                if !previous.public_key.is_before(&signature.public_key) {
                    return Err(Error::BadSignatureOrder);
                }
            }

            let signer_config: SignerConfig = match env
                .storage()
                .instance()
                .get(&DataKey::TypedSignerDetails(signature.public_key.clone()))
            {
                Some(config) => config,
                None => match &signature.public_key {
                    SignerKey::Ed25519(public_key)
                        if env
                            .storage()
                            .instance()
                            .has(&DataKey::Signer(public_key.clone())) =>
                    {
                        SignerConfig {
                            public_key: SignerKey::Ed25519(public_key.clone()),
                            weight: 1,
                            role: None,
                        }
                    }
                    _ => return Err(Error::UnknownSigner),
                },
            };
            signed_weight = signed_weight
                .checked_add(signer_config.weight)
                .ok_or(Error::InvalidPolicy)?;
            authorized_signers.push_back(signer_config.public_key.clone());
            signer_configs.push_back(signer_config);

            match &signature.public_key {
                SignerKey::Ed25519(public_key) => env.crypto().ed25519_verify(
                    public_key,
                    &signature_payload.clone().into(),
                    &signature.signature,
                ),
                SignerKey::Secp256r1(public_key) => env.crypto().secp256r1_verify(
                    public_key,
                    &signature_payload,
                    &signature.signature,
                ),
            }
        }

        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for config in signer_configs.iter() {
            authorized_signers.push_back(config.public_key);
        }

        if signed_weight < policy.weight_threshold {
            return Err(Error::NotEnoughWeight);
        }

        // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
        for requirement in policy.role_requirements.iter() {
            let mut matching_signers = 0u32;
            // loop-bound: ACCEPTED(admin) -- iterates the admin-configured signer set; no on-chain size cap, oversized sets fail deterministically at the CPU limit
            for signer in signer_configs.iter() {
                if signer.role == Some(requirement.role.clone()) {
                    matching_signers += 1;
                }
            }
            if matching_signers < requirement.minimum {
                return Err(Error::RoleQuorumNotMet);
            }
        }

        // Pass 2: the expensive cryptographic verification, run once the
        // policy is known to be satisfiable.
        let payload: Bytes = signature_payload.clone().into();
        for signature in signatures.iter() {
            match &signature.public_key {
                SignerKey::Ed25519(public_key) => {
                    env.crypto()
                        .ed25519_verify(public_key, &payload, &signature.signature)
                }
                SignerKey::Secp256r1(public_key) => env.crypto().secp256r1_verify(
                    public_key,
                    &signature_payload,
                    &signature.signature,
                ),
            }
        }

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AuthorizationApproved {
            timestamp: env.ledger().timestamp(),
            payload,
            signers: authorized_signers,
        }
        .publish(&env);

        Ok(())
    }
}

#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod integration_test;
#[cfg(test)]
mod passkey_vectors_test;
#[cfg(test)]
mod test;
