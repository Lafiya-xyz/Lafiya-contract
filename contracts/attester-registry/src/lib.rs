//! Soroban contract maintaining the allowlist of attesters authorized to
//! call `attest` on the `attestation-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    String, Symbol, Vec,
};

const SCHEMA_VERSION: u32 = 5;
const EVENT_SCHEMA_VERSION: u32 = 2;
const ADMIN_PROPOSAL_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Interface kind reported by `get_interface`, used by clients as a weak
/// identity check when wiring contracts (not proof of authenticity).
pub const CONTRACT_KIND: &str = "lafiya_attester_registry";

/// Version of the public contract interface (functions, errors, types).
/// Bump on any breaking ABI change; `scripts/conformance/check_snapshot.py`
/// refuses a breaking snapshot update without a bump.
pub const INTERFACE_VERSION: u32 = 1;

/// Version of the emitted event schemas (see `docs/events.md`).
pub const EVENT_VERSION: u32 = 1;

/// Optional features this build supports, reported by `get_interface`.
pub const FEATURES: [&str; 6] = [
    "pause",
    "metadata",
    "suspension",
    "batch",
    "max_attesters",
    "migrate",
];

/// Interface and capability metadata returned by `get_interface`, so
/// clients can negotiate features with one call instead of probing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceInfo {
    /// Contract kind, e.g. `lafiya_attester_registry`.
    pub contract_kind: Symbol,
    /// Public interface version; bumped on any breaking ABI change.
    pub interface_version: u32,
    /// Optional features enabled in this build.
    pub features: Vec<Symbol>,
    /// Storage schema version (see `get_schema_version`).
    pub schema_version: u32,
    /// Event schema version.
    pub event_version: u32,
}

/// Storage keys for the attester registry.
///
/// UPGRADE SAFETY: `#[contracttype]` enums serialize variants by their
/// position index, so variant order and existing variants must never change
/// — append new variants at the end only. Reordering breaks decoding of
/// data written by earlier versions.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to add/remove attesters and to upgrade the
    /// contract.
    Admin,
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// Presence of this key (mapped to `AttesterInfo`) means the address is an
    /// allowlisted attester.
    Attester(Address),
    /// Presence of this key means the attester is currently suspended.
    ///
    /// Kept in the enum for storage compatibility. Suspension state is now
    /// stored in `AttesterInfo`.
    Suspended(Address),
    /// The storage schema version of the contract.
    SchemaVersion,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// Soft cap on the number of allowlisted attesters.
    MaxAttesters,
    /// Current count of allowlisted attesters.
    AttesterCount,
    /// Ledger timestamp at which the pending admin proposal expires.
    PendingAdminExpiresAt,
    /// The latest successor address for a rotated attester.
    AttesterRotation(Address),
    /// A point-in-time attester status transition.
    AttesterStatusChange(Address, u32),
    /// Number of recorded status transitions for an attester.
    AttesterStatusChangeCount(Address),
    /// Explicit capabilities granted by the owner.
    Role(Role, Address),
    /// Optional validity bounds for an attester; kept separate so existing
    /// two-field attester records remain readable across upgrades.
    AttesterValidity(Address),
    /// Region and quota assigned to a delegated registrar.
    RegionalRegistrar(Address),
    /// Persistent count of enrolled attesters attributed to a registrar.
    RegionalRegistrarCount(Address),
    /// Registrar responsible for an attester's regional enrollment.
    RegionalAttester(Address),
}

/// Operational capabilities managed by the owner.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Registrar,
    Guardian,
}

/// Emitted when an attester rotates its key while retaining its enrollment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRotated {
    #[topic]
    pub previous_attester: Address,
    #[topic]
    pub new_attester: Address,
}

/// Metadata associated with an allowlisted attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterInfo {
    /// Hash of the attester's off-chain license/credential document, if any.
    pub license_hash: Option<BytesN<32>>,
    /// The geographic region the attester is authorized to attest for, if any.
    /// ISO 3166-2 code, for example `NG-LA`.
    pub region: Option<String>,
    /// Ledger timestamp when the authorization becomes valid, inclusive.
    pub valid_from: Option<u64>,
    /// Ledger timestamp when the authorization expires, exclusive.
    pub valid_until: Option<u64>,
    /// Whether this attester is currently suspended.
    pub suspended: bool,
    /// Whether this address has been removed from the allowlist.
    pub removed: bool,
    /// Why the current suspension was imposed, if any.
    pub suspension_reason: Option<Symbol>,
    /// Ledger timestamp at which the current suspension began.
    pub suspended_since: Option<u64>,
    /// Earliest timestamp whose attestations must no longer be trusted.
    pub trust_revoked_after: Option<u64>,
}

/// A status transition retained for point-in-time allowlist queries.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterStatusChange {
    pub timestamp: u64,
    pub active: bool,
}

/// Region and concurrent enrollment quota granted to a delegated registrar.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionalRegistrarInfo {
    pub region: String,
    pub quota: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredAttesterInfo {
    license_hash: Option<BytesN<32>>,
    region: Option<String>,
    suspended: bool,
    removed: bool,
    suspension_reason: Option<Symbol>,
    suspended_since: Option<u64>,
    trust_revoked_after: Option<u64>,
}

/// Computed authorization state for an allowlisted attester.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttesterStatusKind {
    Active,
    Suspended,
    NotYetValid,
    Expired,
}

/// An allowlisted attester's metadata, suspension state, and computed validity
/// state, as returned by `get_attester_status`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterStatus {
    /// The attester's stored metadata.
    pub info: AttesterInfo,
    /// Whether the attester is currently suspended.
    pub suspended: bool,
    /// Why the attester is suspended, if it is suspended.
    pub suspension_reason: Option<Symbol>,
    /// Ledger timestamp at which the suspension began.
    pub suspended_since: Option<u64>,
    /// Earliest timestamp whose attestations are no longer trusted.
    pub trust_revoked_after: Option<u64>,
    /// Computed status using the current ledger timestamp.
    pub status: AttesterStatusKind,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Default soft cap on the number of allowlisted attesters, used until an
/// admin raises it via `set_max_attesters`. Sized generously above any
/// realistic CHW allowlist so it never trips in normal operation; it exists
/// so a compromised or buggy admin key can't grow persistent-storage rent
/// unboundedly.
const DEFAULT_MAX_ATTESTERS: u32 = 50_000;

/// Maximum number of addresses that may be processed in a single
/// `add_attesters` / `remove_attesters` call.
///
/// Rationale: each address in the batch is one persistent-storage write entry.
/// Soroban's per-transaction write-entry limit is 50, so a ceiling of 40 gives
/// headroom for the instance-storage writes (AttesterCount, Paused, etc.) that
/// happen in the same transaction. Batches larger than this are rejected with
/// `Error::BatchTooLarge` — an early, deterministic error rather than a silent
/// resource-limit abort at the network layer.
pub const BATCH_LIMIT: u32 = 40;
const REGIONAL_BATCH_LIMIT: u32 = 20;
const REGIONAL_REMOVE_BATCH_LIMIT: u32 = 8;

/// Errors returned by the attester registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet; call
    /// `initialize(admin: Address)` before using the contract.
    /// @severity operator
    NotInitialized = 1,
    /// `initialize` was called more than once.
    /// @severity operator
    AlreadyInitialized = 2,
    /// `accept_admin` was called with no pending admin transfer. Admin transfer is a
    /// two-step flow: the current admin must first call `propose_admin` to nominate a
    /// successor, then the nominated address must call `accept_admin` to complete the
    /// transfer. This error is returned when `accept_admin` is called before a
    /// corresponding `propose_admin` call has set a pending admin.
    NoPendingTransfer = 3,
    /// The requested operation is blocked while the contract is paused.
    ContractPaused = 4,
    /// The allowlist is at its configured maximum size. Raise the cap via `set_max_attesters`, or free a slot via `remove_attester`.
    AllowlistFull = 5,
    /// `migrate()` was called while the stored schema version is already
    /// `>= SCHEMA_VERSION`. Only call `migrate()` after `upgrade()` to a
    /// build that bumps `SCHEMA_VERSION`; this error is a safe no-op signal
    /// that there is nothing pending, not a failure to react to.
    /// @severity operator
    MigrationNotRequired = 6,
    /// The referenced attester is not currently allowlisted (never added,
    /// or since removed).
    AttesterNotFound = 7,
    /// The supplied batch exceeds `BATCH_LIMIT` addresses.
    BatchTooLarge = 8,
    /// The proposed admin address is not a valid successor.
    InvalidAdminProposal = 9,
    /// The pending admin proposal has expired.
    ProposalExpired = 10,
    /// The supplied address has not been granted the required role.
    RoleNotGranted = 11,
    /// The attester validity window is empty or reversed.
    InvalidValidityWindow = 12,
    /// The requested attester region is outside the registrar's assigned region.
    RegionMismatch = 13,
    /// The registrar's concurrent enrollment quota has been reached.
    RegionalQuotaExceeded = 14,
    /// The registrar's region cannot change while its attesters remain enrolled.
    RegionalAttestersRemain = 15,
    /// The region is not a valid ISO 3166-2 style code such as `NG-LA`.
    InvalidRegion = 16,
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

/// Emitted once, when the contract is initialized.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an attester is added to the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterAdded {
    #[topic]
    pub attester: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an already-allowlisted attester's metadata is updated via
/// `update_attester_info`. Distinguishable from `AttesterAdded`, which is
/// only emitted on initial enrollment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterInfoUpdated {
    #[topic]
    pub attester: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an attester is removed from the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRemoved {
    #[topic]
    pub attester: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when an attester revokes its own key.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRevoked {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester is suspended.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterSuspended {
    #[topic]
    pub attester: Address,
    pub reason: Symbol,
    pub since: u64,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when a suspended attester is reinstated.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterReinstated {
    #[topic]
    pub attester: Address,
    pub contract_kind: Symbol,
    pub schema_version: u32,
}

/// Emitted when the contract is upgraded to new wasm.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
    pub contract_kind: Symbol,
    pub schema_version: u32,
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

/// The attester registry contract.
#[contract]
pub struct AttesterRegistry;

#[contractimpl]
impl AttesterRegistry {
    /// Set the admin address authorized to manage the allowlist. Can only
    /// be called once; the caller must authorize as the given `admin`.
    pub fn __constructor(env: Env, admin: Address) {
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Initialized {
            admin,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
    }

    /// Rotate an allowlisted attester key without losing metadata or enrollment history.
    /// Both the old and new addresses must authorize the operation.
    pub fn rotate_attester(
        env: Env,
        previous_attester: Address,
        new_attester: Address,
    ) -> Result<(), Error> {
        previous_attester.require_auth();
        new_attester.require_auth();
        Self::require_not_paused(&env)?;
        if previous_attester == new_attester
            || !env
                .storage()
                .persistent()
                .has(&DataKey::Attester(previous_attester.clone()))
            || env
                .storage()
                .persistent()
                .has(&DataKey::Attester(new_attester.clone()))
        {
            return Err(Error::AttesterNotFound);
        }

        let info: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(previous_attester.clone()))
            .filter(|info: &StoredAttesterInfo| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        let suspended = info.suspended;
        let (valid_from, valid_until) = Self::validity(&env, &previous_attester);
        let registrar: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::RegionalAttester(previous_attester.clone()));
        Self::clear_regional_attester(&env, &previous_attester);
        env.storage()
            .persistent()
            .remove(&DataKey::Attester(previous_attester.clone()));
        Self::set_validity(&env, &previous_attester, None, None);
        env.storage()
            .persistent()
            .set(&DataKey::Attester(new_attester.clone()), &info);
        Self::set_validity(&env, &new_attester, valid_from, valid_until);
        if let Some(registrar) = registrar {
            Self::record_regional_attester(&env, &registrar, &new_attester);
        }
        env.storage().persistent().set(
            &DataKey::AttesterRotation(previous_attester.clone()),
            &new_attester,
        );
        Self::record_status_change(&env, &previous_attester, false);
        Self::record_status_change(&env, &new_attester, !suspended);
        AttesterRotated {
            previous_attester,
            new_attester,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Revoke the caller's own attester key immediately. This remains available
    /// while paused so a compromised key can be stopped without admin action.
    pub fn revoke_attester(env: Env, attester: Address) -> Result<(), Error> {
        attester.require_auth();
        let existing = env
            .storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester.clone()))
            .filter(|info| !info.removed);
        if let Some(mut info) = existing {
            info.removed = true;
            info.suspended = false;
            info.suspension_reason = None;
            info.suspended_since = None;
            info.trust_revoked_after = Some(env.ledger().timestamp());
            env.storage()
                .persistent()
                .set(&DataKey::Attester(attester.clone()), &info);
            Self::clear_regional_attester(&env, &attester);
            let count = Self::attester_count(&env);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::AttesterCount, &(count - 1));
            }
            Self::record_status_change(&env, &attester, false);
        }
        AttesterRevoked { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Return the latest address to which `attester` was rotated, if any.
    pub fn get_attester_rotation(env: Env, attester: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::AttesterRotation(attester))
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Grant a registrar or guardian capability. Only the owner may change roles.
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

    /// Revoke a registrar or guardian capability. Only the owner may change roles.
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

    /// Assign a delegated registrar to one region with a concurrent enrollment quota.
    /// Only the owner may create or update this assignment.
    pub fn set_regional_registrar(
        env: Env,
        registrar: Address,
        region: String,
        quota: u32,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::validate_region(&Some(region.clone()))?;
        let key = DataKey::RegionalRegistrar(registrar.clone());
        if let Some(current) = env
            .storage()
            .instance()
            .get::<_, RegionalRegistrarInfo>(&key)
        {
            let count = Self::regional_registrar_count(&env, &registrar);
            if current.region != region && count > 0 {
                return Err(Error::RegionalAttestersRemain);
            }
        }
        env.storage()
            .instance()
            .set(&key, &RegionalRegistrarInfo { region, quota });
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Revoke a delegated registrar assignment. Existing enrollments remain
    /// attributed to the address and continue to count if it is later re-granted.
    pub fn revoke_regional_registrar(env: Env, registrar: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .remove(&DataKey::RegionalRegistrar(registrar));
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Return a registrar's current regional assignment, if any.
    pub fn get_regional_registrar(env: Env, registrar: Address) -> Option<RegionalRegistrarInfo> {
        env.storage()
            .instance()
            .get(&DataKey::RegionalRegistrar(registrar))
    }

    /// Return the number of enrolled attesters attributed to `registrar`.
    pub fn get_regional_registrar_count(env: Env, registrar: Address) -> u32 {
        Self::regional_registrar_count(&env, &registrar)
    }

    /// Propose a new admin address. The caller must authorize as the current admin.
    /// Calling this a second time before `accept_admin` overwrites any pending proposal — the most recent call wins.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::admin(&env)?;
        current_admin.require_auth();
        if new_admin == current_admin || new_admin == env.current_contract_address() {
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
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
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
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

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

    /// Pause the contract, blocking `add_attester`, `add_attester_with_info`,
    /// `update_attester_info`, `remove_attester`, `suspend_attester`,
    /// `reinstate_attester`, and `set_max_attesters` until `unpause` is called.
    /// Requires a Guardian.
    pub fn pause(env: Env, guardian: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Guardian, &guardian)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused {
            by: guardian,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the owner's authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused {
            by: admin,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Add `attester` to the allowlist. Requires a global or regional registrar.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`). A stale
    /// suspension on an address being enrolled is cleared.
    pub fn add_attester(env: Env, registrar: Address, attester: Address) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;
        let region = regional_scope
            .as_ref()
            .map(|assignment| Some(assignment.region.clone()))
            .unwrap_or(None);
        let already_present = env
            .storage()
            .persistent()
            .get::<_, AttesterInfo>(&DataKey::Attester(attester.clone()))
            .map(|info| !info.removed)
            .unwrap_or(false);
        if !already_present {
            Self::ensure_regional_quota(&env, &registrar, regional_scope.as_ref())?;
        }
        if !already_present {
            let count = Self::attester_count(&env);
            let max = Self::max_attesters(&env);
            if count >= max {
                return Err(Error::AllowlistFull);
            }
            env.storage()
                .instance()
                .set(&DataKey::AttesterCount, &(count + 1));
        }
        if already_present {
            let existing: StoredAttesterInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Attester(attester.clone()))
                .ok_or(Error::AttesterNotFound)?;
            if regional_scope.is_some() {
                Self::ensure_regional_existing(
                    &env,
                    &registrar,
                    regional_scope.as_ref(),
                    &attester,
                    &existing,
                )?;
            }
            return Ok(());
        }
        let info = StoredAttesterInfo {
            license_hash: None,
            region,
            suspended: false,
            removed: false,
            suspension_reason: None,
            suspended_since: None,
            trust_revoked_after: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, true);
        if !already_present && regional_scope.is_some() {
            Self::record_regional_attester(&env, &registrar, &attester);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded {
            attester,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Add `attester` with optional metadata to the allowlist. Requires a global
    /// or regional registrar. Regional enrollments inherit the assigned region.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`). If already
    /// allowlisted, this is a no-op; use `update_attester_info` to change
    /// metadata. A stale suspension on an address being enrolled is cleared.
    pub fn add_attester_with_info(
        env: Env,
        registrar: Address,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<String>,
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;
        Self::validate_region(&region)?;
        Self::validate_validity_window(valid_from, valid_until)?;
        let region = Self::scoped_region(regional_scope.as_ref(), region)?;
        let already_present = env
            .storage()
            .persistent()
            .get::<_, AttesterInfo>(&DataKey::Attester(attester.clone()))
            .map(|info| !info.removed)
            .unwrap_or(false);
        if !already_present {
            Self::ensure_regional_quota(&env, &registrar, regional_scope.as_ref())?;
        }
        if !already_present {
            let count = Self::attester_count(&env);
            let max = Self::max_attesters(&env);
            if count >= max {
                return Err(Error::AllowlistFull);
            }
            env.storage()
                .instance()
                .set(&DataKey::AttesterCount, &(count + 1));
        }
        if already_present {
            let existing: StoredAttesterInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Attester(attester.clone()))
                .ok_or(Error::AttesterNotFound)?;
            if regional_scope.is_some() {
                Self::ensure_regional_existing(
                    &env,
                    &registrar,
                    regional_scope.as_ref(),
                    &attester,
                    &existing,
                )?;
            }
            return Ok(());
        }
        let info = StoredAttesterInfo {
            license_hash,
            region,
            suspended: false,
            removed: false,
            suspension_reason: None,
            suspended_since: None,
            trust_revoked_after: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, true);
        Self::set_validity(&env, &attester, valid_from, valid_until);
        if !already_present && regional_scope.is_some() {
            Self::record_regional_attester(&env, &registrar, &attester);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded {
            attester,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Update the metadata of an already-allowlisted `attester`. Requires
    /// a global registrar's authorization. Unlike `add_attester_with_info`, this
    /// never enrolls a new attester: it fails with `Error::AttesterNotFound`
    /// if `attester` is not currently allowlisted (never added, or since
    /// removed), and always emits `AttesterInfoUpdated` rather than
    /// `AttesterAdded`, so profile changes are distinguishable from
    /// enrollment.
    pub fn update_attester_info(
        env: Env,
        registrar: Address,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<String>,
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        Self::validate_region(&region)?;
        Self::validate_validity_window(valid_from, valid_until)?;
        let existing: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))
            .filter(|info: &StoredAttesterInfo| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        if existing.region != region {
            Self::clear_regional_attester(&env, &attester);
        }
        let info = StoredAttesterInfo {
            license_hash,
            region,
            ..existing
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::set_validity(&env, &attester, valid_from, valid_until);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterInfoUpdated {
            attester,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Add multiple attesters to the allowlist in a single transaction.
    ///
    /// Requires a global or regional registrar. Regional registrars are
    /// limited to `REGIONAL_BATCH_LIMIT` entries to bound per-address quota
    /// accounting. Blocked while paused.
    /// Returns `Error::BatchTooLarge` if the applicable batch limit is exceeded.
    /// Returns `Error::AllowlistFull` if adding the new (non-duplicate)
    /// addresses would exceed the configured `max_attesters` cap. Addresses
    /// that are already allowlisted are silently skipped (idempotent), so the
    /// call never fails due to duplicates in the batch and no duplicate events
    /// are emitted. Exactly one `AttesterAdded` event is emitted per newly
    /// added address.
    pub fn add_attesters(
        env: Env,
        registrar: Address,
        attesters: Vec<Address>,
    ) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }
        if regional_scope.is_some() && attesters.len() > REGIONAL_BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        let max = Self::max_attesters(&env);
        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            let already_present = env
                .storage()
                .persistent()
                .get::<_, StoredAttesterInfo>(&key)
                .map(|info| !info.removed)
                .unwrap_or(false);
            if !already_present {
                Self::ensure_regional_quota(&env, &registrar, regional_scope.as_ref())?;
                if count >= max {
                    return Err(Error::AllowlistFull);
                }
                let info = StoredAttesterInfo {
                    license_hash: None,
                    region: regional_scope
                        .as_ref()
                        .map(|assignment| assignment.region.clone()),
                    suspended: false,
                    removed: false,
                    suspension_reason: None,
                    suspended_since: None,
                    trust_revoked_after: None,
                };
                env.storage().persistent().set(&key, &info);
                Self::record_status_change(&env, &attester, true);
                Self::set_validity(&env, &attester, None, None);
                if regional_scope.is_some() {
                    Self::record_regional_attester(&env, &registrar, &attester);
                }
                count += 1;
                AttesterAdded {
                    attester: attester.clone(),
                    contract_kind: Symbol::new(&env, "attester_registry"),
                    schema_version: EVENT_SCHEMA_VERSION,
                }
                .publish(&env);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::AttesterCount, &count);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Remove multiple attesters from the allowlist in a single transaction.
    ///
    /// Requires a global registrar's authorization. Blocked while paused.
    /// Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
    /// If the batch removes regional enrollments, its size is additionally
    /// limited to `REGIONAL_REMOVE_BATCH_LIMIT` to bound storage cleanup.
    /// Addresses that are not currently allowlisted are silently skipped
    /// (idempotent), so the call never fails if an address was already removed
    /// and no spurious events are emitted. Exactly one `AttesterRemoved` event
    /// is emitted per address that was actually removed.
    pub fn remove_attesters(
        env: Env,
        registrar: Address,
        attesters: Vec<Address>,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }
        if attesters.iter().any(|attester| {
            env.storage()
                .persistent()
                .has(&DataKey::RegionalAttester(attester))
        }) && attesters.len() > REGIONAL_REMOVE_BATCH_LIMIT
        {
            return Err(Error::BatchTooLarge);
        }

        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            if let Some(mut info) = env
                .storage()
                .persistent()
                .get::<_, StoredAttesterInfo>(&key)
            {
                if info.removed {
                    continue;
                }
                info.removed = true;
                info.suspended = false;
                info.suspension_reason = None;
                info.suspended_since = None;
                info.trust_revoked_after = Some(env.ledger().timestamp());
                env.storage().persistent().set(&key, &info);
                Self::clear_regional_attester(&env, &attester);
                count = count.saturating_sub(1);
                Self::record_status_change(&env, &attester, false);
                AttesterRemoved {
                    attester: attester.clone(),
                    contract_kind: Symbol::new(&env, "attester_registry"),
                    schema_version: EVENT_SCHEMA_VERSION,
                }
                .publish(&env);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::AttesterCount, &count);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Remove `attester` from the allowlist. Requires a global registrar's
    /// authorization. A no-op if the attester was never allowlisted.
    pub fn remove_attester(env: Env, registrar: Address, attester: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        let mut info = env
            .storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester.clone()));
        let was_present = info.as_ref().map(|info| !info.removed).unwrap_or(false);
        if let Some(ref mut info) = info {
            info.removed = true;
            info.suspended = false;
            info.suspension_reason = None;
            info.suspended_since = None;
            info.trust_revoked_after = Some(env.ledger().timestamp());
            env.storage()
                .persistent()
                .set(&DataKey::Attester(attester.clone()), info);
            Self::clear_regional_attester(&env, &attester);
        }
        if was_present {
            let count = Self::attester_count(&env);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::AttesterCount, &(count - 1));
            }
            Self::record_status_change(&env, &attester, false);
            AttesterRemoved {
                attester: attester.clone(),
                contract_kind: Symbol::new(&env, "attester_registry"),
                schema_version: EVENT_SCHEMA_VERSION,
            }
            .publish(&env);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Set the soft cap on the number of allowlisted attesters. Requires the
    /// admin's authorization. Blocked while the contract is paused. Does not
    /// evict existing attesters if lowered below the current count; it only
    /// blocks further `add_attester` calls.
    pub fn set_max_attesters(env: Env, max_attesters: u32) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MaxAttesters, &max_attesters);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// The current soft cap on the number of allowlisted attesters.
    pub fn get_max_attesters(env: Env) -> u32 {
        Self::max_attesters(&env)
    }

    /// The current number of allowlisted attesters.
    pub fn get_attester_count(env: Env) -> u32 {
        Self::attester_count(&env)
    }

    /// Suspend an allowlisted attester. A global registrar may suspend any
    /// attester; a regional registrar may suspend only attesters in its region.
    ///
    /// Returns `Error::AttesterNotFound` when the address is not allowlisted.
    pub fn suspend_attester(env: Env, registrar: Address, attester: Address) -> Result<(), Error> {
        let reason = Symbol::new(&env, "administrative");
        Self::suspend_attester_with_reason(env, registrar, attester, reason)
    }

    /// Suspend an attester and record the operational reason and cutoff time.
    pub fn suspend_attester_with_reason(
        env: Env,
        registrar: Address,
        attester: Address,
        reason: Symbol,
    ) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;
        let mut info: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))
            .filter(|info: &StoredAttesterInfo| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        if let Some(assignment) = regional_scope {
            if info.region.as_ref() != Some(&assignment.region) {
                return Err(Error::RegionMismatch);
            }
        }
        let since = env.ledger().timestamp();
        info.suspended = true;
        info.suspension_reason = Some(reason.clone());
        info.suspended_since = Some(since);
        info.trust_revoked_after = Some(since);
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, false);
        AttesterSuspended {
            attester,
            reason,
            since,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Reinstate a suspended attester. Requires a global registrar's authorization.
    pub fn reinstate_attester(
        env: Env,
        registrar: Address,
        attester: Address,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        let mut info: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))
            .filter(|info: &StoredAttesterInfo| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        info.suspended = false;
        info.suspension_reason = None;
        info.suspended_since = None;
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, true);
        AttesterReinstated {
            attester,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Whether `attester` is allowlisted, not suspended, and within its validity
    /// window. Callable by anyone, including other contracts.
    pub fn is_attester(env: Env, attester: Address) -> bool {
        let active = env
            .storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester.clone()))
            .map(|info| !info.removed && !info.suspended)
            .unwrap_or(false);
        active && Self::valid_at(Self::validity(&env, &attester), env.ledger().timestamp())
    }

    /// Whether `attester` was active at the supplied ledger timestamp.
    pub fn is_attester_at(env: Env, attester: Address, timestamp: u64) -> bool {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AttesterStatusChangeCount(attester.clone()))
            .unwrap_or(0);
        let mut active = false;
        for sequence in 1..=count {
            let change: Option<AttesterStatusChange> = env
                .storage()
                .persistent()
                .get(&DataKey::AttesterStatusChange(attester.clone(), sequence));
            if let Some(change) = change {
                if change.timestamp > timestamp {
                    break;
                }
                active = change.active;
            }
        }
        active && Self::valid_at(Self::validity(&env, &attester), timestamp)
    }

    /// Whether `attester` is active and authorized for `region`. Attesters
    /// without a configured region remain globally scoped for compatibility.
    pub fn is_attester_for_region(env: Env, attester: Address, region: String) -> bool {
        let Some(info) = Self::attester_info(&env, &attester) else {
            return false;
        };
        Self::is_attester(env, attester) && info.region.is_none_or(|assigned| assigned == region)
    }

    /// Get the optional metadata associated with `attester` if they are allowlisted.
    pub fn get_attester_info(env: Env, attester: Address) -> Option<AttesterInfo> {
        Self::attester_info(&env, &attester)
    }

    /// Return the first timestamp at which attestations by this attester
    /// stopped being trusted.
    pub fn get_attester_trust_revoked_after(env: Env, attester: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester))
            .and_then(|info| info.trust_revoked_after)
    }

    fn validate_region(region: &Option<String>) -> Result<(), Error> {
        if let Some(region) = region {
            let bytes = region.to_bytes();
            if bytes.len() < 4 || bytes.len() > 7 || bytes.get(2) != Some(b'-') {
                return Err(Error::InvalidRegion);
            }
            for (index, byte) in bytes.iter().enumerate() {
                let valid = if index == 2 {
                    byte == b'-'
                } else if index < 2 {
                    byte.is_ascii_uppercase()
                } else {
                    byte.is_ascii_uppercase() || byte.is_ascii_digit()
                };
                if !valid {
                    return Err(Error::InvalidRegion);
                }
            }
        }
        Ok(())
    }

    /// Get `attester`'s metadata with computed validity status. The status
    /// prioritizes suspension, then not-yet-valid and expired windows.
    /// Returns `None` if the attester is not allowlisted.
    pub fn get_attester_status(env: Env, attester: Address) -> Option<AttesterStatus> {
        let info = Self::attester_info(&env, &attester)?;
        let status = Self::status(&info, env.ledger().timestamp());
        Some(AttesterStatus {
            suspended: info.suspended,
            suspension_reason: info.suspension_reason.clone(),
            suspended_since: info.suspended_since,
            trust_revoked_after: info.trust_revoked_after,
            info,
            status,
        })
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

    /// Query the current storage schema version of the contract.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(1)
    }

    /// Upgrade the contract's Wasm code to a new version.
    /// Requires the admin's authorization.
    ///
    /// Runbook:
    /// 1. Build the new Wasm binary (e.g. `cargo build --workspace --release --target wasm32v1-none`).
    /// 2. Upload/install the new Wasm on-chain to obtain its 32-byte hash (`new_wasm_hash`).
    /// 3. The admin calls this `upgrade` function passing the `new_wasm_hash`.
    ///
    /// For any accompanying state/data migrations, see the storage-versioning guidelines
    /// (e.g. implementing migration scripts or handling lazy migrations on reading old schema versions).
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        #[allow(deprecated)]
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        Upgraded {
            new_wasm_hash,
            contract_kind: Symbol::new(&env, "attester_registry"),
            schema_version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Run any pending storage migration, then record the new schema
    /// version. Requires the admin's authorization.
    ///
    /// Call this after `upgrade()` only when the new build bumps
    /// `SCHEMA_VERSION` (a storage-schema-changing release) — including the
    /// first upgrade of a legacy (pre-versioning, schema version `0`)
    /// instance, which must be migrated to version 1. When no migration is
    /// pending (`SchemaVersion >= SCHEMA_VERSION`) this returns
    /// `Error::MigrationNotRequired` so the call can't accidentally re-run.
    pub fn migrate(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();

        let stored = Self::get_schema_version(env.clone());
        if stored >= SCHEMA_VERSION {
            return Err(Error::MigrationNotRequired);
        }

        // Validity bounds use a separate key, so legacy attester records
        // remain readable and need no bulk rewrite.
        let _ = stored;

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    fn validate_validity_window(
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        if let (Some(valid_from), Some(valid_until)) = (valid_from, valid_until) {
            if valid_from >= valid_until {
                return Err(Error::InvalidValidityWindow);
            }
        }
        Ok(())
    }

    fn registrar_scope(
        env: &Env,
        registrar: &Address,
    ) -> Result<Option<RegionalRegistrarInfo>, Error> {
        Self::admin(env)?;
        if env
            .storage()
            .instance()
            .has(&DataKey::Role(Role::Registrar, registrar.clone()))
        {
            registrar.require_auth();
            return Ok(None);
        }

        let assignment = env
            .storage()
            .instance()
            .get(&DataKey::RegionalRegistrar(registrar.clone()))
            .ok_or(Error::RoleNotGranted)?;
        registrar.require_auth();
        Ok(Some(assignment))
    }

    fn scoped_region(
        scope: Option<&RegionalRegistrarInfo>,
        region: Option<String>,
    ) -> Result<Option<String>, Error> {
        match scope {
            Some(assignment) => {
                if region
                    .as_ref()
                    .is_some_and(|region| region != &assignment.region)
                {
                    return Err(Error::RegionMismatch);
                }
                Ok(Some(assignment.region.clone()))
            }
            None => Ok(region),
        }
    }

    fn ensure_regional_existing(
        env: &Env,
        registrar: &Address,
        scope: Option<&RegionalRegistrarInfo>,
        attester: &Address,
        existing: &StoredAttesterInfo,
    ) -> Result<(), Error> {
        let assignment = scope.ok_or(Error::RoleNotGranted)?;
        let assigned_registrar: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::RegionalAttester(attester.clone()));
        if assigned_registrar.as_ref() != Some(registrar)
            || existing.region.as_ref() != Some(&assignment.region)
        {
            return Err(Error::RegionMismatch);
        }
        Ok(())
    }

    fn ensure_regional_quota(
        env: &Env,
        registrar: &Address,
        scope: Option<&RegionalRegistrarInfo>,
    ) -> Result<(), Error> {
        if let Some(assignment) = scope {
            if Self::regional_registrar_count(env, registrar) >= assignment.quota {
                return Err(Error::RegionalQuotaExceeded);
            }
        }
        Ok(())
    }

    fn record_regional_attester(env: &Env, registrar: &Address, attester: &Address) {
        env.storage()
            .persistent()
            .set(&DataKey::RegionalAttester(attester.clone()), registrar);
        let count_key = DataKey::RegionalRegistrarCount(registrar.clone());
        let count = Self::regional_registrar_count(env, registrar);
        env.storage().persistent().set(&count_key, &(count + 1));
        env.storage().persistent().extend_ttl(
            &count_key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    fn clear_regional_attester(env: &Env, attester: &Address) {
        let key = DataKey::RegionalAttester(attester.clone());
        if let Some(registrar) = env.storage().persistent().get::<_, Address>(&key) {
            env.storage().persistent().remove(&key);
            let count = Self::regional_registrar_count(env, &registrar);
            if count > 0 {
                let count_key = DataKey::RegionalRegistrarCount(registrar);
                if count == 1 {
                    env.storage().persistent().remove(&count_key);
                } else {
                    env.storage().persistent().set(&count_key, &(count - 1));
                    env.storage().persistent().extend_ttl(
                        &count_key,
                        INSTANCE_LIFETIME_THRESHOLD,
                        INSTANCE_BUMP_AMOUNT,
                    );
                }
            }
        }
    }

    fn regional_registrar_count(env: &Env, registrar: &Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::RegionalRegistrarCount(registrar.clone()))
            .unwrap_or(0)
    }

    fn validity(env: &Env, attester: &Address) -> (Option<u64>, Option<u64>) {
        env.storage()
            .persistent()
            .get(&DataKey::AttesterValidity(attester.clone()))
            .unwrap_or((None, None))
    }

    fn set_validity(
        env: &Env,
        attester: &Address,
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) {
        let key = DataKey::AttesterValidity(attester.clone());
        if valid_from.is_none() && valid_until.is_none() {
            env.storage().persistent().remove(&key);
        } else {
            env.storage()
                .persistent()
                .set(&key, &(valid_from, valid_until));
        }
    }

    fn attester_info(env: &Env, attester: &Address) -> Option<AttesterInfo> {
        let stored: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))?;
        if stored.removed {
            return None;
        }
        let (valid_from, valid_until) = Self::validity(env, attester);
        Some(AttesterInfo {
            license_hash: stored.license_hash,
            region: stored.region,
            valid_from,
            valid_until,
            suspended: stored.suspended,
            removed: stored.removed,
            suspension_reason: stored.suspension_reason,
            suspended_since: stored.suspended_since,
            trust_revoked_after: stored.trust_revoked_after,
        })
    }

    fn valid_at(validity: (Option<u64>, Option<u64>), timestamp: u64) -> bool {
        let (valid_from, valid_until) = validity;
        valid_from.is_none_or(|start| timestamp >= start)
            && valid_until.is_none_or(|end| timestamp < end)
    }

    fn status(info: &AttesterInfo, timestamp: u64) -> AttesterStatusKind {
        if info.suspended {
            AttesterStatusKind::Suspended
        } else if info.valid_from.is_some_and(|start| timestamp < start) {
            AttesterStatusKind::NotYetValid
        } else if info.valid_until.is_some_and(|end| timestamp >= end) {
            AttesterStatusKind::Expired
        } else {
            AttesterStatusKind::Active
        }
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

    fn max_attesters(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxAttesters)
            .unwrap_or(DEFAULT_MAX_ATTESTERS)
    }

    fn attester_count(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::AttesterCount)
            .unwrap_or(0)
    }

    fn record_status_change(env: &Env, attester: &Address, active: bool) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AttesterStatusChangeCount(attester.clone()))
            .unwrap_or(0);
        let sequence = count + 1;
        env.storage().persistent().set(
            &DataKey::AttesterStatusChange(attester.clone(), sequence),
            &AttesterStatusChange {
                timestamp: env.ledger().timestamp(),
                active,
            },
        );
        env.storage().persistent().set(
            &DataKey::AttesterStatusChangeCount(attester.clone()),
            &sequence,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod enumeration_spike;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod large_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
