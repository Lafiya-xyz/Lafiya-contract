//! Soroban contract recording attestations of off-chain records, gated by
//! allowlist membership in the `attester-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address,
    BytesN, Env, Symbol, Vec,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
    fn get_attester_trust_revoked_after(env: Env, attester: Address) -> Option<u64>;
}

/// Emitted when the current admin nominates a successor.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferProposed {
    #[topic]
    pub current_admin: Address,
    #[topic]
    pub proposed_admin: Address,
    pub expires_at: u64,
}

/// Emitted when a pending admin transfer is cancelled.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferCancelled {
    #[topic]
    pub admin: Address,
    #[topic]
    pub proposed_admin: Address,
}

/// Maximum number of attestations to keep per record hash. At capacity,
/// further attestations require admin revocation rather than evicting history.
const MAX_HISTORY: u64 = 10;
const ADMIN_PROPOSAL_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

const CONSENT_VALIDITY_SECONDS: u64 = 7 * 24 * 60 * 60;
const CONSENT_TTL_THRESHOLD: u32 = 60_480;
const CONSENT_TTL_BUMP: u32 = 120_960;

const SCHEMA_VERSION: u32 = 2;

/// Interface kind reported by `get_interface`, used by clients as a weak
/// identity check when wiring contracts (not proof of authenticity).
pub const CONTRACT_KIND: &str = "lafiya_attestation_registry";

/// Version of the public contract interface (functions, errors, types).
/// Bump on any breaking ABI change; `scripts/conformance/check_snapshot.py`
/// refuses a breaking snapshot update without a bump.
pub const INTERFACE_VERSION: u32 = 1;

/// Version of the emitted event schemas (see `docs/events.md`).
pub const EVENT_VERSION: u32 = 1;

/// Optional features this build supports, reported by `get_interface`.
pub const FEATURES: [&str; 4] = ["pause", "history", "revocation", "repoint_registry"];

/// Interface and capability metadata returned by `get_interface`, so
/// clients can negotiate features with one call instead of probing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceInfo {
    /// Contract kind, e.g. `lafiya_attestation_registry`.
    pub contract_kind: Symbol,
    /// Public interface version; bumped on any breaking ABI change.
    pub interface_version: u32,
    /// Optional features enabled in this build.
    pub features: Vec<Symbol>,
    /// Storage schema version (stored `SchemaVersion`, default 1).
    pub schema_version: u32,
    /// Event schema version.
    pub event_version: u32,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Upper bound on a rate-limit window (30 days of ledgers). Keeps the
/// temporary `RateWindow` entry's TTL well inside the network's maximum
/// entry TTL.
const MAX_RATE_WINDOW_LEDGERS: u32 = 518_400;

/// Storage keys for the attestation registry.
///
/// UPGRADE SAFETY: `#[contracttype]` enums serialize variants by their
/// position index, so variant order and existing variants must never change
/// — append new variants at the end only. Reordering breaks decoding of
/// data written by earlier versions.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to (re)point `AttesterRegistry` and to upgrade
    /// the contract.
    Admin,
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// The deployed `attester-registry` contract consulted on every `attest` call.
    AttesterRegistry,
    /// Attestation for a given record hash at a specific sequence number.
    Attestation(BytesN<32>, u64),
    /// Latest sequence number for a given record hash.
    AttestationSequence(BytesN<32>),
    /// Count of attestations for a given record hash (for bounded history).
    AttestationCount(BytesN<32>),
    /// The storage schema version of the contract.
    SchemaVersion,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// Ledger timestamp at which the pending admin proposal expires.
    PendingAdminExpiresAt,
    /// The global per-attester `RateLimit` (instance storage). Absent means
    /// attestations are not rate limited.
    RateLimit,
    /// Per-attester override of `RateLimit.max_per_window` (persistent storage).
    RateLimitOverride(Address),
    /// The attester's current `RateWindow` (temporary storage; expires with
    /// the window).
    RateWindow(Address),
}

/// Admin-configured per-attester attestation rate limit: at most
/// `max_per_window` attestations per attester in any window of
/// `window_ledgers` ledgers.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimit {
    pub max_per_window: u32,
    pub window_ledgers: u32,
}

/// An attester's fixed rate-limit window: the ledger it started at and the
/// number of attestations recorded in it so far.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateWindow {
    pub window_start_ledger: u32,
    pub count: u32,
}

/// A single attestation: proof that `attester` verified the off-chain
/// record whose hash is the lookup key, at `timestamp`. Never contains the
/// underlying health data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    /// The allowlisted attester that verified the record.
    pub attester: Address,
    /// Ledger timestamp at which the attestation was recorded.
    pub timestamp: u64,
    /// Commitment scheme version: `0` is legacy/unversioned, `1` is LRC-1.
    pub commitment_version: u32,
}

/// Metadata for a batch of record commitments anchored by one attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationBatch {
    /// The allowlisted attester authorizing every leaf in the batch.
    pub attester: Address,
    /// Ledger timestamp at which the root was anchored.
    pub timestamp: u64,
    /// Number of record commitments represented by the Merkle root.
    pub leaf_count: u32,
}

/// An attestation together with its freshness status. Attestations written
/// before expiry tracking was introduced have no expiry and are treated stale.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationStatus {
    /// The attestation being checked.
    pub attestation: Attestation,
    /// Timestamp selected by the patient when authorizing this attestation.
    pub expires_at: Option<u64>,
    /// Whether the expiry has passed, or is unknown for a legacy attestation.
    pub is_expired: bool,
}

/// Emitted when admin ownership finishes transferring to a new address.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
}

/// Emitted when a new attestation is recorded for a record hash.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRecorded {
    #[topic]
    pub record_hash: BytesN<32>,
    /// The allowlisted attester that verified the record.
    pub attester: Address,
    /// Ledger timestamp at which the attestation was recorded.
    pub timestamp: u64,
    /// Patient-selected timestamp after which responders should treat this
    /// verification as stale.
    pub expires_at: u64,
}

/// Emitted when an attestation is revoked.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRevoked {
    #[topic]
    pub record_hash: BytesN<32>,
}

/// Emitted when an attester records the last attestation its rate-limit
/// window allows. Published at most once per attester per window, so it
/// cannot itself be used to spam; further attempts in the window fail with
/// `Error::RateLimited` and, as failed invocations, publish no events.
#[contractevent]
#[derive(Clone, Debug)]
pub struct RateLimitHit {
    #[topic]
    pub attester: Address,
    /// First ledger at which the attester may attest again.
    pub retry_after_ledger: u32,
}

/// Emitted when the admin changes the global attestation rate limit.
#[contractevent]
#[derive(Clone, Debug)]
pub struct RateLimitSet {
    /// Maximum attestations per attester per window; `0` disables limiting.
    pub max_per_window: u32,
    pub window_ledgers: u32,
}

/// Emitted when state-changing operations are paused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Paused {
    #[topic]
    pub by: Address,
}

/// Emitted when state-changing operations are unpaused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Unpaused {
    #[topic]
    pub by: Address,
}

/// Emitted when the `attester-registry` contract this registry consults is repointed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRegistryRepointed {
    #[topic]
    pub previous: Address,
    #[topic]
    pub new: Address,
}

/// Errors returned by the attestation registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet.
    /// @severity operator
    NotInitialized = 1,
    /// `initialize` was called more than once.
    /// @severity operator
    AlreadyInitialized = 2,
    /// The caller is not allowlisted by the `attester-registry` contract.
    AttesterNotAllowlisted = 3,
    /// `accept_admin` was called with no pending admin transfer. Admin transfer is a
    /// two-step flow: the current admin must first call `propose_admin` to nominate a
    /// successor, then the nominated address must call `accept_admin` to complete the
    /// transfer. This error is returned when `accept_admin` is called before a
    /// corresponding `propose_admin` call has set a pending admin.
    NoPendingTransfer = 4,
    /// The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration.
    /// @severity operator
    InvalidRegistryWiring = 5,
    /// No attestation exists for the given record hash / sequence.
    AttestationNotFound = 6,
    /// The requested operation is blocked while the contract is paused.
    ContractPaused = 7,
    /// The proposed admin address is not a valid successor.
    InvalidAdminProposal = 8,
    /// The pending admin proposal has expired.
    ProposalExpired = 9,
    /// The attester has used up its rate-limit window. Call
    /// `get_rate_limit_retry_after` for the first ledger it may attest again.
    RateLimited = 10,
    /// `window_ledgers` was `0` or longer than 30 days of ledgers.
    InvalidRateLimit = 11,
}

// Source-provenance metadata (SEP-46 `contractmetav0`, keys per SEP-55
// "Contract Build Verification"). Deterministic for a given commit:
// LAFIYA_GIT_COMMIT is injected by build.rs, see docs/releasing.md.
soroban_sdk::contractmeta!(
    key = "source_repo",
    val = "github:Lafiya-xyz/Lafiya-contract"
);
soroban_sdk::contractmeta!(key = "home_domain", val = "lafiya-xyz.github.io");
soroban_sdk::contractmeta!(key = "crate_name", val = env!("CARGO_PKG_NAME"));
soroban_sdk::contractmeta!(key = "crate_version", val = env!("CARGO_PKG_VERSION"));
soroban_sdk::contractmeta!(key = "source_rev", val = env!("LAFIYA_GIT_COMMIT"));

/// The attestation registry contract.
#[contract]
pub struct AttestationRegistry;

#[contractimpl]
impl AttestationRegistry {
    /// Set the admin and the `attester-registry` contract this registry
    /// consults for allowlist checks. Can only be called once; the caller
    /// must authorize as the given `admin`.
    ///
    /// ## Best-effort interface check
    ///
    /// This function performs a lightweight sanity check against
    /// `attester_registry`: it calls `is_attester_for_region` with a
    /// throwaway address and region, and confirms the call does not trap.
    /// This confirms the address implements the expected interface — it does
    /// **not** prove the address is the canonical, trusted deployment.
    pub fn initialize(env: Env, admin: Address, attester_registry: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        // Best-effort sanity check: verify attester_registry implements the
        // regional allowlist interface with a throwaway address.
        let registry = AttesterRegistryClient::new(&env, &attester_registry);
        // The current contract address is valid but will not be allowlisted.
        let throwaway = env.current_contract_address();
        let throwaway_region = Symbol::new(&env, "interface");
        if registry
            .try_is_attester_for_region(&throwaway, &throwaway_region)
            .is_err()
        {
            return Err(Error::InvalidRegistryWiring);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(())
    }

    /// Cancel the pending admin transfer. Requires the current admin's authorization.
    pub fn cancel_admin_proposal(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        let proposed_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingTransfer)?;
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.storage()
            .instance()
            .remove(&DataKey::PendingAdminExpiresAt);
        AdminTransferCancelled {
            admin,
            proposed_admin,
        }
        .publish(&env);
        Ok(())
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Grant a guardian or revoker capability. Only the owner may change roles.
    pub fn grant_role(env: Env, role: Role, account: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Role(role, account.clone()), &true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Revoke a guardian or revoker capability. Only the owner may change roles.
    pub fn revoke_role(env: Env, role: Role, account: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .remove(&DataKey::Role(role, account.clone()));
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Return whether `account` holds `role`.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        env.storage().instance().has(&DataKey::Role(role, account))
    }

    /// Return the configured attester-registry contract address.
    pub fn get_attester_registry(env: Env) -> Result<Address, Error> {
        Self::attester_registry(&env)
    }

    /// Query the storage schema version for this contract instance.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(1)
    }

    /// Mark the additive version-2 storage schema as available after upgrade.
    /// No data reshaping is required; all newly introduced keys are optional
    /// until the corresponding operation first writes them.
    pub fn migrate(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();

        if Self::get_schema_version(env.clone()) >= SCHEMA_VERSION {
            return Err(Error::MigrationNotRequired);
        }

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Propose a new admin address. The caller must authorize as the current admin.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::admin(&env)?;
        current_admin.require_auth();
        let current_registry = Self::attester_registry(&env)?;
        if new_admin == current_admin
            || new_admin == env.current_contract_address()
            || new_admin == current_registry
        {
            return Err(Error::InvalidAdminProposal);
        }
        let expires_at = env
            .ledger()
            .timestamp()
            .saturating_add(ADMIN_PROPOSAL_TTL_SECONDS);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdminExpiresAt, &expires_at);
        AdminTransferProposed {
            current_admin,
            proposed_admin: new_admin,
            expires_at,
        }
        .publish(&env);
        Ok(())
    }

    /// Accept the proposed admin transfer. The caller must authorize as the pending admin.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let previous_admin = Self::admin(&env)?;
        let pending_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingTransfer)?;
        let expires_at: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdminExpiresAt)
            .unwrap_or(0);
        if env.ledger().timestamp() > expires_at {
            env.storage().instance().remove(&DataKey::PendingAdmin);
            env.storage()
                .instance()
                .remove(&DataKey::PendingAdminExpiresAt);
            return Err(Error::ProposalExpired);
        }

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.storage()
            .instance()
            .remove(&DataKey::PendingAdminExpiresAt);

        AdminTransferred {
            previous_admin,
            new_admin: pending_admin,
        }
        .publish(&env);

        Ok(())
    }

    /// Change the attester-registry contract this registry consults for
    /// allowlist checks. Requires the admin's authorization and a compatible
    /// regional allowlist interface. Emits
    /// `AttesterRegistryRepointed` for indexer/audit visibility.
    pub fn set_attester_registry(env: Env, new_registry: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let registry = AttesterRegistryClient::new(&env, &new_registry);
        let throwaway = env.current_contract_address();
        let throwaway_region = Symbol::new(&env, "interface");
        if registry
            .try_is_attester_for_region(&throwaway, &throwaway_region)
            .is_err()
        {
            return Err(Error::InvalidRegistryWiring);
        }

        let previous = Self::attester_registry(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &new_registry);

        AttesterRegistryRepointed {
            previous,
            new: new_registry,
        }
        .publish(&env);

        Ok(())
    }

    /// Pause the contract, blocking `attest` until `unpause` is called.
    /// Requires the Guardian role.
    pub fn pause(env: Env, guardian: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Guardian, &guardian)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: guardian }.publish(&env);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the owner's authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused { by: admin }.publish(&env);
        Ok(())
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Grant a one-time authorization for `attester` to attest `record_hash`.
    /// The patient must authorize this call. The grant is bound to the patient,
    /// attester, and record hash, expires after seven days, and is consumed by
    /// the matching `attest` call. The patient also selects when that
    /// attestation becomes stale.
    pub fn consent_attestation(
        env: Env,
        patient: Address,
        attester: Address,
        record_hash: BytesN<32>,
        attestation_expires_at: u64,
    ) -> Result<(), Error> {
        patient.require_auth();
        Self::require_not_paused(&env)?;

        if attestation_expires_at <= env.ledger().timestamp() {
            return Err(Error::InvalidAttestationExpiry);
        }

        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

        let expires_at = env
            .ledger()
            .timestamp()
            .checked_add(CONSENT_VALIDITY_SECONDS)
            .ok_or(Error::TimestampOverflow)?;
        let key = DataKey::PatientConsent(record_hash.clone(), patient.clone(), attester.clone());
        env.storage().persistent().set(&key, &expires_at);
        let validity_key = DataKey::PatientConsentValidity(record_hash, patient, attester);
        env.storage()
            .persistent()
            .set(&validity_key, &attestation_expires_at);
        env.storage()
            .persistent()
            .extend_ttl(&key, CONSENT_TTL_THRESHOLD, CONSENT_TTL_BUMP);
        env.storage().persistent().extend_ttl(
            &validity_key,
            CONSENT_TTL_THRESHOLD,
            CONSENT_TTL_BUMP,
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Record that `attester` verified the record hashing to `record_hash`.
    /// Requires `attester`'s authorization, an unexpired one-time patient
    /// consent grant for this exact patient/attester/hash tuple, and that
    /// `attester` is currently allowlisted in the configured registry.
    /// Stores the attestation with an incrementing sequence number,
    /// maintaining a bounded history (MAX_HISTORY entries per hash).
    pub fn attest(
        env: Env,
        attester: Address,
        patient: Address,
        record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        Self::require_allowlisted_attester(&env, &attester)?;
        Self::record_attestation(&env, attester, record_hash, None)
    }

    /// Record a verification and explicitly link this record hash to its
    /// previous version. The previous hash must already have attestations.
    pub fn attest_version(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
        previous_record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        Self::record_attestation(&env, attester, record_hash, 0)
    }

    /// Record an attestation with an explicit one-byte commitment scheme
    /// version. `0` is reserved for legacy/unversioned commitments, `1` is
    /// LRC-1, and future values may identify later schemes. As with `attest`,
    /// a relayer may submit the transaction and pay its fees using an
    /// authorization entry signed by `attester`.
    pub fn attest_versioned(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
        commitment_version: u32,
    ) -> Result<Attestation, Error> {
        if commitment_version > u8::MAX as u32 {
            return Err(Error::InvalidCommitmentVersion);
        }
        Self::record_attestation(&env, attester, record_hash, commitment_version)
    }

    fn record_attestation(
        env: &Env,
        attester: Address,
        record_hash: BytesN<32>,
        commitment_version: u32,
    ) -> Result<Attestation, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        Self::require_allowlisted_attester(&env, &attester)?;
        Self::record_attestation(&env, attester, record_hash, Some(previous_record_hash))
    }

    /// Record up to `BATCH_LIMIT` attestations in one transaction. Each
    /// attester must authorize their request; allowlist status is checked once
    /// per distinct attester in the batch.
    pub fn batch_attest(
        env: Env,
        requests: Vec<AttestationRequest>,
    ) -> Result<Vec<Attestation>, Error> {
        if requests.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }
        Self::require_not_paused(&env)?;

        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        // A failing call into the admin-configured registry should abort the
        // attestation, so the panicking (non-`try_`) client is intended here.
        // nosemgrep: soroban-panicking-cross-contract-call
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

        Self::consume_rate_limit(&env, &attester)?;

        let attestation = Attestation {
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
            commitment_version,
        };

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .unwrap_or(0);
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let consent_key =
            DataKey::PatientConsent(record_hash.clone(), patient.clone(), attester.clone());
        let consent_expires_at: u64 = env
            .storage()
            .persistent()
            .get(&consent_key)
            .ok_or(Error::PatientConsentRequired)?;
        if env.ledger().timestamp() >= consent_expires_at {
            return Err(Error::PatientConsentExpired);
        }
        let consent_validity_key =
            DataKey::PatientConsentValidity(record_hash.clone(), patient, attester.clone());
        let attestation_expires_at: u64 = env
            .storage()
            .persistent()
            .get(&consent_validity_key)
            .ok_or(Error::PatientConsentRequired)?;
        if env.ledger().timestamp() >= attestation_expires_at {
            return Err(Error::InvalidAttestationExpiry);
        }
        env.storage().persistent().remove(&consent_key);
        env.storage().persistent().remove(&consent_validity_key);

        let attestation = Attestation {
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
        };

        let new_sequence = sequence + 1;

        env.storage()
            .persistent()
            .set(&DataKey::RecordRevoked(record_hash.clone()), &true);
        env.storage().persistent().set(
            &DataKey::RevokedThroughSequence(record_hash.clone()),
            &sequence,
        );
        env.storage().persistent().set(
            &DataKey::AttestationExpiry(record_hash.clone(), new_sequence),
            &attestation_expires_at,
        );

        env.storage().persistent().set(
            &DataKey::AttestationSequence(record_hash.clone()),
            &new_sequence,
        );

        let new_count = count + 1;

        if new_count > MAX_HISTORY {
            let oldest_sequence = new_count.saturating_sub(MAX_HISTORY);
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), oldest_sequence));
        }

        env.storage()
            .persistent()
            .set(&DataKey::AttestationCount(record_hash.clone()), &new_count);

        // Extend TTL on the specific attestation entry just written, so it is
        // not subject to state-archival independently of the instance storage.
        env.storage().persistent().extend_ttl(
            &DataKey::Attestation(record_hash.clone(), new_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::AttestationExpiry(record_hash.clone(), new_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );

        for seq in (start_sequence..=sequence).rev() {
            if seq <= revoked_through {
                continue;
            }
            let key = DataKey::Attestation(record_hash.clone(), seq);
            let Some(attestation) = env.storage().persistent().get::<_, Attestation>(&key) else {
                continue;
            };
            if attestation.attester == attester
                && !env
                    .storage()
                    .persistent()
                    .has(&DataKey::AttestationWithdrawn(record_hash.clone(), seq))
            {
                env.storage().persistent().set(
                    &DataKey::AttestationWithdrawn(record_hash.clone(), seq),
                    &true,
                );
                found = true;
            }
        }

        AttestationRecorded {
            record_hash,
            attester,
            timestamp: attestation.timestamp,
            expires_at: attestation_expires_at,
        }
    }

    /// Look up the latest active attestation for `record_hash`, if any. Callable
    /// by anyone — this is what lets a responder's QR scan independently
    /// check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::RecordRevoked(record_hash.clone()))
            .unwrap_or(false)
        {
            return None;
        }
        Self::latest_active_attestation(&env, record_hash)
    }

    /// Return whether a record hash is unverified, verified, withdrawn, or
    /// explicitly revoked. This remains informative after admin revocation.
    pub fn get_attestation_status(env: Env, record_hash: BytesN<32>) -> AttestationStatus {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::RecordRevoked(record_hash.clone()))
            .unwrap_or(false)
        {
            return AttestationStatus::Revoked;
        }
        if Self::latest_active_attestation(&env, record_hash.clone()).is_some() {
            return AttestationStatus::Verified;
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::AttestationSequence(record_hash))
        {
            return AttestationStatus::Withdrawn;
        }
        AttestationStatus::NeverAttested
    }

    /// Return whether `attester` has an active, withdrawn, or revoked
    /// verification for `record_hash`.
    pub fn get_attester_attestation_status(
        env: Env,
        record_hash: BytesN<32>,
        attester: Address,
    ) -> AttesterAttestationStatus {
        if env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .ok_or(Error::AttestationNotFound)?;

        let Some(sequence) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::AttestationSequence(record_hash.clone()))
        else {
            return AttesterAttestationStatus::NeverAttested;
        };
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let start_sequence = if count > MAX_HISTORY {
            sequence.saturating_sub(MAX_HISTORY - 1)
        } else {
            1
        };
        let revoked_through: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RevokedThroughSequence(record_hash.clone()))
            .unwrap_or(0);
        let mut found = false;
        let mut withdrawn = false;

        for seq in start_sequence..=sequence {
            let Some(attestation) = env
                .storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), seq));
            env.storage()
                .persistent()
                .remove(&DataKey::AttestationExpiry(record_hash.clone(), seq));
        }

        if withdrawn {
            AttesterAttestationStatus::Withdrawn
        } else if found {
            AttesterAttestationStatus::Revoked
        } else {
            AttesterAttestationStatus::NeverAttested
        }
    }

    /// Return the explicitly linked previous record version, if any.
    pub fn get_previous_record_hash(env: Env, record_hash: BytesN<32>) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKey::PreviousRecordHash(record_hash))
    }

    /// Return the explicitly linked next record version, if any.
    pub fn get_next_record_hash(env: Env, record_hash: BytesN<32>) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKey::NextRecordHash(record_hash))
    }

    /// Report the contract kind, interface version, enabled features, and
    /// storage/event schema versions for runtime compatibility negotiation.
    /// Callable by anyone.
    pub fn get_interface(env: Env) -> InterfaceInfo {
        let mut features = Vec::new(&env);
        for feature in FEATURES {
            features.push_back(Symbol::new(&env, feature));
        }
        InterfaceInfo {
            contract_kind: Symbol::new(&env, CONTRACT_KIND),
            interface_version: INTERFACE_VERSION,
            features,
            schema_version: env
                .storage()
                .instance()
                .get(&DataKey::SchemaVersion)
                .unwrap_or(1),
            event_version: EVENT_VERSION,
        }
    }

    /// Look up the full attestation history for `record_hash`, if any.
    /// Returns attestations in chronological order (oldest first).
    /// Callable by anyone.
    pub fn get_attestation_history(env: Env, record_hash: BytesN<32>) -> Vec<Attestation> {
        let sequence: u64 = match env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
        {
            Some(seq) => seq,
            None => return Vec::new(&env),
        };

        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);

        let mut history = Vec::new(&env);
        let start_sequence = if count > MAX_HISTORY {
            sequence.saturating_sub(MAX_HISTORY - 1)
        } else {
            1
        };

        for seq in start_sequence..=sequence {
            if let Some(attestation) = env
                .storage()
                .persistent()
                .get(&DataKey::Attestation(record_hash.clone(), seq))
            {
                history.push_back(attestation);
            }
        }

        history
    }

    /// Set the global per-attester attestation rate limit: at most
    /// `max_per_window` attestations per attester per `window_ledgers`
    /// ledgers. `max_per_window == 0` disables rate limiting. Requires the
    /// admin's authorization.
    pub fn set_attestation_rate_limit(
        env: Env,
        max_per_window: u32,
        window_ledgers: u32,
    ) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        if max_per_window == 0 {
            env.storage().instance().remove(&DataKey::RateLimit);
        } else {
            if window_ledgers == 0 || window_ledgers > MAX_RATE_WINDOW_LEDGERS {
                return Err(Error::InvalidRateLimit);
            }
            env.storage().instance().set(
                &DataKey::RateLimit,
                &RateLimit {
                    max_per_window,
                    window_ledgers,
                },
            );
        }
        RateLimitSet {
            max_per_window,
            window_ledgers,
        }
        .publish(&env);
        Ok(())
    }

    /// Return the global attestation rate limit, if one is configured.
    pub fn get_attestation_rate_limit(env: Env) -> Option<RateLimit> {
        env.storage().instance().get(&DataKey::RateLimit)
    }

    /// Override `max_per_window` for a single attester (e.g. a high-volume
    /// clinical site). The global window length still applies. Requires the
    /// admin's authorization.
    pub fn set_attester_rate_limit(
        env: Env,
        attester: Address,
        max_per_window: u32,
    ) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        let key = DataKey::RateLimitOverride(attester);
        env.storage().persistent().set(&key, &max_per_window);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        Ok(())
    }

    /// Remove an attester's rate-limit override, reverting it to the global
    /// limit. Requires the admin's authorization.
    pub fn remove_attester_rate_limit(env: Env, attester: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::RateLimitOverride(attester));
        Ok(())
    }

    /// If `attester` has used up its current rate-limit window, return the
    /// first ledger at which it may attest again; otherwise `None`.
    pub fn get_rate_limit_retry_after(env: Env, attester: Address) -> Option<u32> {
        let limit: RateLimit = env.storage().instance().get(&DataKey::RateLimit)?;
        let max = Self::max_per_window(&env, &attester, &limit);
        let window = Self::current_window(&env, &attester, &limit)?;
        if window.count >= max {
            Some(
                window
                    .window_start_ledger
                    .saturating_add(limit.window_ledgers),
            )
        } else {
            None
        }
    }

    fn max_per_window(env: &Env, attester: &Address, limit: &RateLimit) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::RateLimitOverride(attester.clone()))
            .unwrap_or(limit.max_per_window)
    }

    /// The attester's still-open window, or `None` if it has rolled over or
    /// its temporary entry has expired.
    fn current_window(env: &Env, attester: &Address, limit: &RateLimit) -> Option<RateWindow> {
        let window: RateWindow = env
            .storage()
            .temporary()
            .get(&DataKey::RateWindow(attester.clone()))?;
        let now = env.ledger().sequence();
        if now
            >= window
                .window_start_ledger
                .saturating_add(limit.window_ledgers)
        {
            None
        } else {
            Some(window)
        }
    }

    /// Count one attestation against `attester`'s window, or fail with
    /// `RateLimited` if the window is full. The window lives in temporary
    /// storage; if the entry is archived early the attester simply starts a
    /// fresh window (fails open), which is acceptable for rate limiting.
    fn consume_rate_limit(env: &Env, attester: &Address) -> Result<(), Error> {
        let limit: RateLimit = match env.storage().instance().get(&DataKey::RateLimit) {
            Some(limit) => limit,
            None => return Ok(()),
        };
        let max = Self::max_per_window(env, attester, &limit);
        let now = env.ledger().sequence();
        let mut window = Self::current_window(env, attester, &limit).unwrap_or(RateWindow {
            window_start_ledger: now,
            count: 0,
        });
        if window.count >= max {
            return Err(Error::RateLimited);
        }
        window.count += 1;

        let window_end = window
            .window_start_ledger
            .saturating_add(limit.window_ledgers);
        let key = DataKey::RateWindow(attester.clone());
        env.storage().temporary().set(&key, &window);
        let remaining = window_end - now;
        env.storage()
            .temporary()
            .extend_ttl(&key, remaining, remaining);

        if window.count == max {
            RateLimitHit {
                attester: attester.clone(),
                retry_after_ledger: window_end,
            }
            .publish(env);
        }
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn require_role(env: &Env, role: Role, account: &Address) -> Result<(), Error> {
        if !env
            .storage()
            .instance()
            .has(&DataKey::Role(role, account.clone()))
        {
            return Err(Error::RoleNotGranted);
        }
        account.require_auth();
        Ok(())
    }

    fn attester_registry(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)
    }

    fn require_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }

    fn require_allowlisted_attester(env: &Env, attester: &Address) -> Result<(), Error> {
        let registry_id = Self::attester_registry(env)?;
        let registry = AttesterRegistryClient::new(env, &registry_id);
        if registry.is_attester(attester) {
            Ok(())
        } else {
            Err(Error::AttesterNotAllowlisted)
        }
    }

    fn record_attestation(
        env: &Env,
        attester: Address,
        record_hash: BytesN<32>,
        previous_record_hash: Option<BytesN<32>>,
    ) -> Result<Attestation, Error> {
        let new_previous_link = if let Some(previous_hash) = previous_record_hash {
            if previous_hash == record_hash
                || !env
                    .storage()
                    .persistent()
                    .has(&DataKey::AttestationSequence(previous_hash.clone()))
            {
                return Err(Error::InvalidRecordVersion);
            }

            let existing_previous: Option<BytesN<32>> = env
                .storage()
                .persistent()
                .get(&DataKey::PreviousRecordHash(record_hash.clone()));
            let existing_next: Option<BytesN<32>> = env
                .storage()
                .persistent()
                .get(&DataKey::NextRecordHash(previous_hash.clone()));
            let current_next: Option<BytesN<32>> = env
                .storage()
                .persistent()
                .get(&DataKey::NextRecordHash(record_hash.clone()));
            if existing_previous
                .as_ref()
                .is_some_and(|existing| existing != &previous_hash)
                || existing_next
                    .as_ref()
                    .is_some_and(|existing| existing != &record_hash)
                || (existing_previous.is_none() && current_next.is_some())
            {
                return Err(Error::InvalidRecordVersion);
            }
            existing_previous.is_none().then_some(previous_hash)
        } else {
            None
        };

        let attestation = Attestation {
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
        };

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .unwrap_or(0);
        let new_sequence = sequence + 1;

        env.storage().persistent().set(
            &DataKey::Attestation(record_hash.clone(), new_sequence),
            &attestation,
        );
        env.storage().persistent().set(
            &DataKey::AttestationSequence(record_hash.clone()),
            &new_sequence,
        );

        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let new_count = count + 1;
        if new_count > MAX_HISTORY {
            let oldest_sequence = new_count.saturating_sub(MAX_HISTORY);
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), oldest_sequence));
        }
        env.storage()
            .persistent()
            .set(&DataKey::AttestationCount(record_hash.clone()), &new_count);
        env.storage()
            .persistent()
            .remove(&DataKey::RecordRevoked(record_hash.clone()));

        env.storage().persistent().extend_ttl(
            &DataKey::Attestation(record_hash.clone(), new_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        if let Some(previous_hash) = new_previous_link {
            env.storage().persistent().set(
                &DataKey::PreviousRecordHash(record_hash.clone()),
                &previous_hash,
            );
            env.storage().persistent().set(
                &DataKey::NextRecordHash(previous_hash.clone()),
                &record_hash,
            );
            env.storage().persistent().extend_ttl(
                &DataKey::PreviousRecordHash(record_hash.clone()),
                INSTANCE_LIFETIME_THRESHOLD,
                INSTANCE_BUMP_AMOUNT,
            );
            env.storage().persistent().extend_ttl(
                &DataKey::NextRecordHash(previous_hash.clone()),
                INSTANCE_LIFETIME_THRESHOLD,
                INSTANCE_BUMP_AMOUNT,
            );
            RecordVersionLinked {
                previous_record_hash: previous_hash,
                record_hash: record_hash.clone(),
            }
            .publish(env);
        }

        AttestationRecorded {
            record_hash,
            attester,
            timestamp: attestation.timestamp,
        }
        .publish(env);

        Ok(attestation)
    }

    fn latest_active_attestation(env: &Env, record_hash: BytesN<32>) -> Option<Attestation> {
        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))?;
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let start_sequence = if count > MAX_HISTORY {
            sequence.saturating_sub(MAX_HISTORY - 1)
        } else {
            1
        };
        let revoked_through: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RevokedThroughSequence(record_hash.clone()))
            .unwrap_or(0);

        for seq in (start_sequence..=sequence).rev() {
            if seq <= revoked_through
                || env
                    .storage()
                    .persistent()
                    .has(&DataKey::AttestationWithdrawn(record_hash.clone(), seq))
            {
                continue;
            }
            if let Some(attestation) = env
                .storage()
                .persistent()
                .get(&DataKey::Attestation(record_hash.clone(), seq))
            {
                return Some(attestation);
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod fuzz_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
