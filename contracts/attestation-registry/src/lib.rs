//! Soroban contract recording attestations of off-chain records, gated by
//! allowlist membership in the `attester-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    panic_with_error, Address, BytesN, Env, String, Symbol, Vec,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
    fn is_attester_for_region(env: Env, attester: Address, region: String) -> bool;
    fn get_attester_trust_revoked_after(env: Env, attester: Address) -> Option<u64>;
}

/// Maximum number of distinct attester slots retained per record hash.
/// Repeat submissions refresh an attester's existing slot; a new attester
/// beyond the limit evicts the oldest slot (FIFO).
const MAX_HISTORY: u64 = 10;

/// Maximum number of requests accepted by one `batch_attest` call.
pub const BATCH_LIMIT: u32 = 50;

/// Default maximum age (seconds) of an attestation reported as verified.
pub const DEFAULT_MAX_ATTESTATION_AGE: u64 = 31_536_000;

const ADMIN_PROPOSAL_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

const CONSENT_VALIDITY_SECONDS: u64 = 7 * 24 * 60 * 60;
const CONSENT_TTL_THRESHOLD: u32 = 60_480;
const CONSENT_TTL_BUMP: u32 = 120_960;

/// Storage schema version of this build.
pub const SCHEMA_VERSION: u32 = 3;
/// Schema version stamped on emitted audit events.
pub const EVENT_SCHEMA_VERSION: u32 = 2;

/// Interface kind reported by `get_interface`, used by clients as a weak
/// identity check when wiring contracts (not proof of authenticity).
pub const CONTRACT_KIND: &str = "lafiya_attestation_registry";

/// Version of the public contract interface (functions, errors, types).
/// Bump on any breaking ABI change; `scripts/conformance/check_snapshot.py`
/// refuses a breaking snapshot update without a bump.
pub const INTERFACE_VERSION: u32 = 2;

/// Version of the emitted event schemas (see `docs/events.md`).
pub const EVENT_VERSION: u32 = 2;

/// Optional features this build supports, reported by `get_interface`.
pub const FEATURES: [&str; 10] = [
    "pause",
    "history",
    "revocation",
    "repoint_registry",
    "roles",
    "consent",
    "withdrawal",
    "batch",
    "merkle_anchor",
    "rate_limit",
];

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

/// Operational capabilities managed by the owner.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// May pause the contract.
    Guardian,
    /// May revoke all attestations for a record hash.
    Revoker,
}

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
    /// Attestation slot for a record hash at a specific sequence number.
    Attestation(BytesN<32>, u64),
    /// Highest slot sequence number allocated for a record hash.
    AttestationSequence(BytesN<32>),
    /// Reserved legacy key. Retained at its enum position for storage
    /// compatibility; history windows are now derived from the sequence.
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
    /// Explicit capabilities granted by the owner.
    Role(Role, Address),
    /// Maximum age of an attestation that may be reported as verified.
    MaxAttestationAge,
    /// A patient's one-time authorization for an attester and record hash.
    PatientConsent(BytesN<32>, Address, Address),
    /// The patient-selected expiry for an attestation consent grant.
    PatientConsentValidity(BytesN<32>, Address, Address),
    /// Expiry timestamp for an attestation at a specific sequence number.
    AttestationExpiry(BytesN<32>, u64),
    /// Metadata for an anchored Merkle batch, keyed by its root.
    AttestationBatch(BytesN<32>),
    /// Whether an attester withdrew a specific retained attestation.
    AttestationWithdrawn(BytesN<32>, u64),
    /// Whether all attestations for a record hash were revoked by a revoker.
    RecordRevoked(BytesN<32>),
    /// Highest attestation sequence covered by a revocation.
    RevokedThroughSequence(BytesN<32>),
    /// Previous version hash for a record hash.
    PreviousRecordHash(BytesN<32>),
    /// Next version hash for a record hash.
    NextRecordHash(BytesN<32>),
}

/// Revoker-configured per-attester attestation rate limit: at most
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

/// One attestation to submit in a batch, optionally linked to a previous
/// record version. Each request needs the patient's matching consent.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AttestationRequest {
    pub attester: Address,
    pub patient: Address,
    pub record_hash: BytesN<32>,
    pub previous_record_hash: Option<BytesN<32>>,
}

/// Summary state for a record hash, distinguishing absent verification from
/// an explicit withdrawal or revocation.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordStatus {
    NeverAttested,
    Verified,
    Withdrawn,
    Revoked,
}

/// Status of one attester's verification for a record hash.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttesterAttestationStatus {
    NeverAttested,
    Active,
    Withdrawn,
    Revoked,
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

/// Emitted when admin ownership finishes transferring to a new address.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
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
    /// Commitment scheme version of the attested record.
    pub commitment_version: u32,
    /// Sequence number of the retained slot written by this event.
    pub sequence: u64,
    /// Sequence number evicted from the bounded FIFO history, if any.
    pub evicted_sequence: Option<u64>,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an attester withdraws their own attestation.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationWithdrawn {
    #[topic]
    pub record_hash: BytesN<32>,
    #[topic]
    pub attester: Address,
}

/// Emitted when a new record hash is linked to its previous version.
#[contractevent]
#[derive(Clone, Debug)]
pub struct RecordVersionLinked {
    #[topic]
    pub previous_record_hash: BytesN<32>,
    #[topic]
    pub record_hash: BytesN<32>,
}

/// Emitted when an attestation is revoked.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRevoked {
    #[topic]
    pub record_hash: BytesN<32>,
    /// Revoker who authorized the revocation.
    pub by: Address,
    /// Number of active attestations that were revoked.
    pub removed_count: u64,
    /// Non-sensitive, concise reason code supplied by the revoker.
    pub reason: Symbol,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an attester anchors a Merkle root for multiple records.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationBatchAnchored {
    #[topic]
    pub root: BytesN<32>,
    /// The allowlisted attester authorizing every leaf in the batch.
    pub attester: Address,
    /// Ledger timestamp at which the root was anchored.
    pub timestamp: u64,
    /// Number of record commitments represented by the Merkle root.
    pub leaf_count: u32,
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

/// Emitted when the contract is upgraded to new wasm.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
}

/// Emitted when state-changing operations are paused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Paused {
    #[topic]
    pub by: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when state-changing operations are unpaused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Unpaused {
    #[topic]
    pub by: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when the `attester-registry` contract this registry consults is repointed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRegistryRepointed {
    #[topic]
    pub previous: Address,
    #[topic]
    pub new: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Errors returned by the attestation registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Required registry configuration is missing from storage.
    /// @severity operator
    NotInitialized = 1,
    /// Reserved for compatibility with the removed public initializer.
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
    /// The supplied address has not been granted the required role.
    RoleNotGranted = 12,
    /// `migrate()` was called when no storage migration is pending.
    MigrationNotRequired = 13,
    /// No unexpired patient consent grant matches this attester, patient and record hash.
    PatientConsentRequired = 14,
    /// The patient's consent grant has expired.
    PatientConsentExpired = 15,
    /// The current ledger timestamp cannot be safely advanced to calculate a time bound.
    TimestampOverflow = 16,
    /// The patient-selected attestation expiry is not in the future.
    InvalidAttestationExpiry = 17,
    /// The configured attester-registry could not be called.
    AttesterRegistryUnavailable = 18,
    /// A Merkle batch must contain at least one record.
    EmptyBatch = 19,
    /// The Merkle root has already been anchored.
    BatchAlreadyAnchored = 20,
    /// The attester has no active attestation for the given record hash.
    AttestationNotOwned = 21,
    /// The supplied previous hash or version relationship is invalid.
    InvalidRecordVersion = 22,
    /// The batch contains more requests than the supported maximum.
    BatchTooLarge = 23,
    /// The commitment scheme version does not fit in one byte.
    InvalidCommitmentVersion = 24,
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
    /// Configure the admin and `attester-registry` atomically at deployment.
    /// The caller must authorize as the given `admin`.
    ///
    /// ## Best-effort interface check
    ///
    /// This function performs a lightweight sanity check against
    /// `attester_registry`: it calls `is_attester_for_region` with a
    /// throwaway address and region, and confirms the call does not trap.
    /// This confirms the address implements the expected interface — it does
    /// **not** prove the address is the canonical, trusted deployment.
    pub fn __constructor(env: Env, admin: Address, attester_registry: Address) {
        admin.require_auth();

        if !Self::registry_is_compatible(&env, &attester_registry) {
            panic_with_error!(&env, Error::InvalidRegistryWiring);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
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
            .set(&DataKey::Role(role, account), &true);
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
            .remove(&DataKey::Role(role, account));
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

    /// Complete a pending storage migration after an upgrade. The schema
    /// changes since version 1 are additive (new optional keys), so no data
    /// needs to be reshaped; this only records the new version. Requires the
    /// admin's authorization.
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

    /// Upgrade the contract's Wasm code. Only the owner may authorize upgrades.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        #[allow(deprecated)]
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        Upgraded { new_wasm_hash }.publish(&env);
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
            contract_kind: Symbol::new(&env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);

        Ok(())
    }

    /// Change the attester-registry contract this registry consults for
    /// allowlist checks. Requires the admin's authorization and a compatible
    /// regional allowlist interface. Emits `AttesterRegistryRepointed` for
    /// indexer/audit visibility.
    pub fn set_attester_registry(env: Env, new_registry: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        if !Self::registry_is_compatible(&env, &new_registry) {
            return Err(Error::InvalidRegistryWiring);
        }

        let previous = Self::attester_registry(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &new_registry);

        AttesterRegistryRepointed {
            previous,
            new: new_registry,
            contract_kind: Symbol::new(&env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);

        Ok(())
    }

    /// Pause the contract, blocking `attest` until `unpause` is called.
    /// Requires the Guardian role.
    pub fn pause(env: Env, guardian: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Guardian, &guardian)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused {
            by: guardian,
            contract_kind: Symbol::new(&env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the owner's authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused {
            by: admin,
            contract_kind: Symbol::new(&env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Set the maximum age in seconds for which an attestation is considered
    /// current. Requires the admin's authorization.
    pub fn set_max_attestation_age(env: Env, max_age: u64) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::MaxAttestationAge, &max_age);
        Ok(())
    }

    /// Return the maximum age in seconds for which an attestation is
    /// considered current. Defaults to 365 days.
    pub fn get_max_attestation_age(env: Env) -> u64 {
        Self::max_attestation_age(&env)
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

        Self::require_allowlisted_attester(&env, &attester)?;

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
    /// Stores one retained slot per attester and record hash; re-attesting
    /// refreshes the attester's slot without evicting other attesters.
    ///
    /// A relayer may submit the transaction and pay its fees using an
    /// authorization entry signed by `attester`.
    pub fn attest(
        env: Env,
        attester: Address,
        patient: Address,
        record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        Self::require_allowlisted_attester(&env, &attester)?;
        Self::record_attestation(&env, attester, patient, record_hash, 0, None)
    }

    /// Record an attestation with an explicit one-byte commitment scheme
    /// version. `0` is reserved for legacy/unversioned commitments, `1` is
    /// LRC-1, and future values may identify later schemes. As with `attest`,
    /// a relayer may submit the transaction.
    pub fn attest_versioned(
        env: Env,
        attester: Address,
        patient: Address,
        record_hash: BytesN<32>,
        commitment_version: u32,
    ) -> Result<Attestation, Error> {
        if commitment_version > u8::MAX as u32 {
            return Err(Error::InvalidCommitmentVersion);
        }
        attester.require_auth();
        Self::require_not_paused(&env)?;
        Self::require_allowlisted_attester(&env, &attester)?;
        Self::record_attestation(
            &env,
            attester,
            patient,
            record_hash,
            commitment_version,
            None,
        )
    }

    /// Record a verification and explicitly link this record hash to its
    /// previous version. The previous hash must already have attestations.
    pub fn attest_version(
        env: Env,
        attester: Address,
        patient: Address,
        record_hash: BytesN<32>,
        previous_record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        Self::require_allowlisted_attester(&env, &attester)?;
        Self::record_attestation(
            &env,
            attester,
            patient,
            record_hash,
            0,
            Some(previous_record_hash),
        )
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

        let mut checked_attesters: Vec<Address> = Vec::new(&env);
        let mut results = Vec::new(&env);

        for request in requests.iter() {
            if !checked_attesters.contains(&request.attester) {
                request.attester.require_auth();
                Self::require_allowlisted_attester(&env, &request.attester)?;
                checked_attesters.push_back(request.attester.clone());
            }

            let attestation = Self::record_attestation(
                &env,
                request.attester,
                request.patient,
                request.record_hash,
                0,
                request.previous_record_hash,
            )?;
            results.push_back(attestation);
        }

        Ok(results)
    }

    /// Anchor a Merkle root containing multiple record commitments.
    ///
    /// The attester authorizes the root and leaf count, while the contract
    /// stores only one persistent entry for the entire batch. Verifiers
    /// reconstruct inclusion proofs using the tree format documented in
    /// `docs/merkle-batch-attestations.md`.
    pub fn anchor_batch(
        env: Env,
        attester: Address,
        root: BytesN<32>,
        leaf_count: u32,
    ) -> Result<AttestationBatch, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;

        if leaf_count == 0 {
            return Err(Error::EmptyBatch);
        }

        Self::require_allowlisted_attester(&env, &attester)?;

        let key = DataKey::AttestationBatch(root.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::BatchAlreadyAnchored);
        }

        let batch = AttestationBatch {
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
            leaf_count,
        };
        env.storage().persistent().set(&key, &batch);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AttestationBatchAnchored {
            root,
            attester,
            timestamp: batch.timestamp,
            leaf_count,
        }
        .publish(&env);

        Ok(batch)
    }

    /// Look up metadata for a Merkle batch root, if it has been anchored.
    pub fn get_attestation_batch(env: Env, root: BytesN<32>) -> Option<AttestationBatch> {
        env.storage()
            .persistent()
            .get(&DataKey::AttestationBatch(root))
    }

    /// Revoke all attestations for `record_hash` without erasing the
    /// historical entries. Requires the Revoker role. `reason` is a concise,
    /// non-sensitive code recorded in the `AttestationRevoked` event.
    pub fn revoke_attestation(
        env: Env,
        revoker: Address,
        record_hash: BytesN<32>,
        reason: Symbol,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Revoker, &revoker)?;

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .ok_or(Error::AttestationNotFound)?;

        let mut removed_count = 0u64;
        for seq in Self::history_start(sequence)..=sequence {
            if Self::slot_is_active(&env, &record_hash, seq) {
                removed_count += 1;
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::RecordRevoked(record_hash.clone()), &true);
        env.storage().persistent().set(
            &DataKey::RevokedThroughSequence(record_hash.clone()),
            &sequence,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::RecordRevoked(record_hash.clone()),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::RevokedThroughSequence(record_hash.clone()),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );

        AttestationRevoked {
            record_hash,
            by: revoker,
            removed_count,
            reason,
            contract_kind: Symbol::new(&env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);

        Ok(())
    }

    /// Withdraw all of the caller's active attestations for `record_hash`.
    /// Other attesters' attestations are unaffected.
    pub fn withdraw_attestation(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
    ) -> Result<(), Error> {
        attester.require_auth();

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .ok_or(Error::AttestationNotOwned)?;

        let mut found = false;
        for seq in Self::history_start(sequence)..=sequence {
            if !Self::slot_is_active(&env, &record_hash, seq) {
                continue;
            }
            let owned = env
                .storage()
                .persistent()
                .get::<_, Attestation>(&DataKey::Attestation(record_hash.clone(), seq))
                .is_some_and(|attestation| attestation.attester == attester);
            if owned {
                let key = DataKey::AttestationWithdrawn(record_hash.clone(), seq);
                env.storage().persistent().set(&key, &true);
                env.storage().persistent().extend_ttl(
                    &key,
                    INSTANCE_LIFETIME_THRESHOLD,
                    INSTANCE_BUMP_AMOUNT,
                );
                found = true;
            }
        }

        if !found {
            return Err(Error::AttestationNotOwned);
        }

        AttestationWithdrawn {
            record_hash,
            attester,
        }
        .publish(&env);
        Ok(())
    }

    /// Return whether the latest active attestation is current and backed by
    /// an attester who remains allowlisted and unsuspended.
    ///
    /// An attestation is verified only when it exists, has not been revoked or
    /// withdrawn, is no older than `get_max_attestation_age`, has not passed
    /// its patient-selected expiry, and its attester is currently active in
    /// the configured attester-registry. A registry call failure returns
    /// `AttesterRegistryUnavailable`.
    pub fn is_verified(env: Env, record_hash: BytesN<32>) -> Result<bool, Error> {
        let Some((sequence, attestation)) = Self::latest_active(&env, &record_hash) else {
            return Ok(false);
        };
        let now = env.ledger().timestamp();
        if attestation.timestamp > now
            || now - attestation.timestamp > Self::max_attestation_age(&env)
        {
            return Ok(false);
        }
        let expiry: Option<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationExpiry(record_hash, sequence));
        if expiry.is_some_and(|expires_at| now >= expires_at) {
            return Ok(false);
        }

        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        match registry.try_is_attester(&attestation.attester) {
            Ok(Ok(is_attester)) => Ok(is_attester),
            _ => Err(Error::AttesterRegistryUnavailable),
        }
    }

    /// Whether the latest active attestation was made before its attester's
    /// trust cutoff. Raw attestations remain available through
    /// `get_attestation` for auditability.
    pub fn is_attestation_trusted(env: Env, record_hash: BytesN<32>) -> bool {
        let Some((_, attestation)) = Self::latest_active(&env, &record_hash) else {
            return false;
        };
        let Ok(registry_id) = Self::attester_registry(&env) else {
            return false;
        };
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        match registry.try_get_attester_trust_revoked_after(&attestation.attester) {
            Ok(Ok(Some(cutoff))) => attestation.timestamp < cutoff,
            Ok(Ok(None)) => true,
            _ => false,
        }
    }

    /// Look up the latest active attestation for `record_hash`, if any.
    /// Callable by anyone — this is what lets a responder's QR scan
    /// independently check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        Self::latest_active(&env, &record_hash).map(|(_, attestation)| attestation)
    }

    /// Look up the latest active attestation for each record hash in input
    /// order. Each output entry corresponds to the hash at the same input
    /// position; missing attestations are `None`. Callable by anyone.
    pub fn get_attestations(env: Env, record_hashes: Vec<BytesN<32>>) -> Vec<Option<Attestation>> {
        let mut attestations = Vec::new(&env);
        for record_hash in record_hashes {
            attestations.push_back(
                Self::latest_active(&env, &record_hash).map(|(_, attestation)| attestation),
            );
        }
        attestations
    }

    /// Look up the latest active attestation with its patient-selected
    /// expiry status. An attestation without a recorded expiry is treated as
    /// stale.
    pub fn get_attestation_status(env: Env, record_hash: BytesN<32>) -> Option<AttestationStatus> {
        let (sequence, attestation) = Self::latest_active(&env, &record_hash)?;
        Some(Self::attestation_status(
            &env,
            &record_hash,
            sequence,
            attestation,
        ))
    }

    /// Return whether a record hash is unverified, verified, withdrawn, or
    /// explicitly revoked. This remains informative after revocation.
    pub fn get_record_status(env: Env, record_hash: BytesN<32>) -> RecordStatus {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::RecordRevoked(record_hash.clone()))
            .unwrap_or(false)
        {
            return RecordStatus::Revoked;
        }
        if Self::latest_active(&env, &record_hash).is_some() {
            return RecordStatus::Verified;
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::AttestationSequence(record_hash))
        {
            return RecordStatus::Withdrawn;
        }
        RecordStatus::NeverAttested
    }

    /// Return whether `attester` has an active, withdrawn, or revoked
    /// verification for `record_hash`.
    pub fn get_attester_attestation_status(
        env: Env,
        record_hash: BytesN<32>,
        attester: Address,
    ) -> AttesterAttestationStatus {
        let Some(sequence) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::AttestationSequence(record_hash.clone()))
        else {
            return AttesterAttestationStatus::NeverAttested;
        };
        let revoked_through = Self::revoked_through(&env, &record_hash);
        let mut revoked = false;
        let mut withdrawn = false;

        for seq in (Self::history_start(sequence)..=sequence).rev() {
            let Some(attestation) = env
                .storage()
                .persistent()
                .get::<_, Attestation>(&DataKey::Attestation(record_hash.clone(), seq))
            else {
                continue;
            };
            if attestation.attester != attester {
                continue;
            }
            if seq <= revoked_through {
                revoked = true;
            } else if env
                .storage()
                .persistent()
                .has(&DataKey::AttestationWithdrawn(record_hash.clone(), seq))
            {
                withdrawn = true;
            } else {
                return AttesterAttestationStatus::Active;
            }
        }

        if withdrawn {
            AttesterAttestationStatus::Withdrawn
        } else if revoked {
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
            schema_version: Self::get_schema_version(env.clone()),
            event_version: EVENT_VERSION,
        }
    }

    /// Look up the retained attestation history for `record_hash`, including
    /// withdrawn and revoked entries, ordered by the first submission of each
    /// retained attester slot. Callable by anyone.
    pub fn get_attestation_history(env: Env, record_hash: BytesN<32>) -> Vec<Attestation> {
        let mut history = Vec::new(&env);
        let Some(sequence) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::AttestationSequence(record_hash.clone()))
        else {
            return history;
        };

        for seq in Self::history_start(sequence)..=sequence {
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

    /// Look up the retained attestation history with expiry status for each
    /// entry. Attestations without an expiry are treated stale.
    pub fn get_attestation_history_status(
        env: Env,
        record_hash: BytesN<32>,
    ) -> Vec<AttestationStatus> {
        let mut history = Vec::new(&env);
        let Some(sequence) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::AttestationSequence(record_hash.clone()))
        else {
            return history;
        };

        for seq in Self::history_start(sequence)..=sequence {
            if let Some(attestation) = env
                .storage()
                .persistent()
                .get(&DataKey::Attestation(record_hash.clone(), seq))
            {
                history.push_back(Self::attestation_status(
                    &env,
                    &record_hash,
                    seq,
                    attestation,
                ));
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
        Self::admin(env)?;
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

    /// Best-effort interface check: the candidate registry must answer the
    /// regional allowlist query for a throwaway address without trapping.
    fn registry_is_compatible(env: &Env, registry_id: &Address) -> bool {
        let registry = AttesterRegistryClient::new(env, registry_id);
        let throwaway = env.current_contract_address();
        let region = String::from_str(env, "NG-LA");
        registry
            .try_is_attester_for_region(&throwaway, &region)
            .is_ok()
    }

    fn max_attestation_age(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MaxAttestationAge)
            .unwrap_or(DEFAULT_MAX_ATTESTATION_AGE)
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
        match registry.try_is_attester(attester) {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(Error::AttesterNotAllowlisted),
            _ => Err(Error::AttesterRegistryUnavailable),
        }
    }

    /// First retained slot sequence for a record whose newest slot is `sequence`.
    fn history_start(sequence: u64) -> u64 {
        sequence.saturating_sub(MAX_HISTORY - 1).max(1)
    }

    fn revoked_through(env: &Env, record_hash: &BytesN<32>) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::RevokedThroughSequence(record_hash.clone()))
            .unwrap_or(0)
    }

    /// Whether the slot exists and is neither revoked nor withdrawn.
    fn slot_is_active(env: &Env, record_hash: &BytesN<32>, seq: u64) -> bool {
        seq > Self::revoked_through(env, record_hash)
            && env
                .storage()
                .persistent()
                .has(&DataKey::Attestation(record_hash.clone(), seq))
            && !env
                .storage()
                .persistent()
                .has(&DataKey::AttestationWithdrawn(record_hash.clone(), seq))
    }

    /// The active slot with the newest timestamp (highest sequence wins ties).
    fn latest_active(env: &Env, record_hash: &BytesN<32>) -> Option<(u64, Attestation)> {
        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))?;
        let revoked_through = Self::revoked_through(env, record_hash);

        let mut best: Option<(u64, Attestation)> = None;
        for seq in (Self::history_start(sequence)..=sequence).rev() {
            if seq <= revoked_through
                || env
                    .storage()
                    .persistent()
                    .has(&DataKey::AttestationWithdrawn(record_hash.clone(), seq))
            {
                continue;
            }
            let Some(attestation) = env
                .storage()
                .persistent()
                .get::<_, Attestation>(&DataKey::Attestation(record_hash.clone(), seq))
            else {
                continue;
            };
            let newer = best
                .as_ref()
                .is_none_or(|(_, current)| attestation.timestamp > current.timestamp);
            if newer {
                best = Some((seq, attestation));
            }
        }
        best
    }

    fn attestation_status(
        env: &Env,
        record_hash: &BytesN<32>,
        sequence: u64,
        attestation: Attestation,
    ) -> AttestationStatus {
        let expires_at: Option<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationExpiry(record_hash.clone(), sequence));
        let is_expired = expires_at
            .map(|expiry| env.ledger().timestamp() >= expiry)
            .unwrap_or(true);
        AttestationStatus {
            attestation,
            expires_at,
            is_expired,
        }
    }

    /// Validate and store the optional previous-version link. Returns the
    /// previous hash when a new link must be written.
    fn check_version_link(
        env: &Env,
        record_hash: &BytesN<32>,
        previous_record_hash: Option<BytesN<32>>,
    ) -> Result<Option<BytesN<32>>, Error> {
        let Some(previous_hash) = previous_record_hash else {
            return Ok(None);
        };
        if &previous_hash == record_hash
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
                .is_some_and(|existing| existing != record_hash)
            || (existing_previous.is_none() && current_next.is_some())
        {
            return Err(Error::InvalidRecordVersion);
        }
        Ok(existing_previous.is_none().then_some(previous_hash))
    }

    /// Consume the patient's consent grant for this attester and record,
    /// returning the patient-selected attestation expiry.
    fn consume_consent(
        env: &Env,
        attester: &Address,
        patient: Address,
        record_hash: &BytesN<32>,
    ) -> Result<u64, Error> {
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
        let validity_key =
            DataKey::PatientConsentValidity(record_hash.clone(), patient, attester.clone());
        let attestation_expires_at: u64 = env
            .storage()
            .persistent()
            .get(&validity_key)
            .ok_or(Error::PatientConsentRequired)?;
        if env.ledger().timestamp() >= attestation_expires_at {
            return Err(Error::InvalidAttestationExpiry);
        }
        env.storage().persistent().remove(&consent_key);
        env.storage().persistent().remove(&validity_key);
        Ok(attestation_expires_at)
    }

    /// Shared write path for `attest`, `attest_versioned`, `attest_version`
    /// and `batch_attest`. Callers have already checked the attester's
    /// authorization, the pause flag and allowlist membership.
    fn record_attestation(
        env: &Env,
        attester: Address,
        patient: Address,
        record_hash: BytesN<32>,
        commitment_version: u32,
        previous_record_hash: Option<BytesN<32>>,
    ) -> Result<Attestation, Error> {
        let new_previous_link = Self::check_version_link(env, &record_hash, previous_record_hash)?;
        Self::consume_rate_limit(env, &attester)?;
        let attestation_expires_at = Self::consume_consent(env, &attester, patient, &record_hash)?;

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
        let revoked_through = Self::revoked_through(env, &record_hash);

        // Refresh this attester's existing live slot, if it has one.
        let existing_sequence = if sequence == 0 {
            None
        } else {
            (Self::history_start(sequence)..=sequence).find(|seq| {
                *seq > revoked_through
                    && env
                        .storage()
                        .persistent()
                        .get::<_, Attestation>(&DataKey::Attestation(record_hash.clone(), *seq))
                        .is_some_and(|existing| existing.attester == attester)
            })
        };

        let mut evicted_sequence = None;
        let stored_sequence = match existing_sequence {
            Some(existing_sequence) => {
                env.storage()
                    .persistent()
                    .remove(&DataKey::AttestationWithdrawn(
                        record_hash.clone(),
                        existing_sequence,
                    ));
                existing_sequence
            }
            None => {
                let new_sequence = sequence + 1;
                env.storage().persistent().set(
                    &DataKey::AttestationSequence(record_hash.clone()),
                    &new_sequence,
                );
                if new_sequence > MAX_HISTORY {
                    let oldest_sequence = new_sequence - MAX_HISTORY;
                    env.storage()
                        .persistent()
                        .remove(&DataKey::Attestation(record_hash.clone(), oldest_sequence));
                    env.storage()
                        .persistent()
                        .remove(&DataKey::AttestationExpiry(
                            record_hash.clone(),
                            oldest_sequence,
                        ));
                    env.storage()
                        .persistent()
                        .remove(&DataKey::AttestationWithdrawn(
                            record_hash.clone(),
                            oldest_sequence,
                        ));
                    evicted_sequence = Some(oldest_sequence);
                }
                new_sequence
            }
        };

        env.storage().persistent().set(
            &DataKey::Attestation(record_hash.clone(), stored_sequence),
            &attestation,
        );
        env.storage().persistent().set(
            &DataKey::AttestationExpiry(record_hash.clone(), stored_sequence),
            &attestation_expires_at,
        );
        env.storage()
            .persistent()
            .remove(&DataKey::RecordRevoked(record_hash.clone()));

        // Keep the written entries alive independently of instance storage.
        env.storage().persistent().extend_ttl(
            &DataKey::Attestation(record_hash.clone(), stored_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::AttestationExpiry(record_hash.clone(), stored_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::AttestationSequence(record_hash.clone()),
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
            expires_at: attestation_expires_at,
            commitment_version,
            sequence: stored_sequence,
            evicted_sequence,
            contract_kind: Symbol::new(env, "attestation_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(env);

        Ok(attestation)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod fuzz_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
