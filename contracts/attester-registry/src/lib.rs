//! Soroban contract maintaining the allowlist of attesters authorized to
//! call `attest` on the `attestation-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    Symbol, Vec,
};

const SCHEMA_VERSION: u32 = 4;

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
    Suspended(Address),
    /// The storage schema version of the contract.
    SchemaVersion,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// Soft cap on the number of allowlisted attesters.
    MaxAttesters,
    /// Current count of allowlisted attesters.
    AttesterCount,
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

/// Metadata associated with an allowlisted attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterInfo {
    /// Hash of the attester's off-chain license/credential document, if any.
    pub license_hash: Option<BytesN<32>>,
    /// The geographic region the attester is authorized to attest for, if any.
    pub region: Option<Symbol>,
    /// Ledger timestamp when the authorization becomes valid, inclusive.
    pub valid_from: Option<u64>,
    /// Ledger timestamp when the authorization expires, exclusive.
    pub valid_until: Option<u64>,
}

/// Region and concurrent enrollment quota granted to a delegated registrar.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionalRegistrarInfo {
    pub region: Symbol,
    pub quota: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredAttesterInfo {
    license_hash: Option<BytesN<32>>,
    region: Option<Symbol>,
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

/// An allowlisted attester's metadata together with its current suspension
/// state, as returned by `get_attester_status`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterStatus {
    /// The attester's stored metadata.
    pub info: AttesterInfo,
    /// Whether the attester is currently suspended.
    pub suspended: bool,
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

/// Errors returned by the attester registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet; call
    /// `initialize(admin: Address)` before using the contract.
    NotInitialized = 1,
    /// `initialize` was called more than once.
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
    MigrationNotRequired = 6,
    /// The referenced attester is not currently allowlisted (never added,
    /// or since removed).
    AttesterNotFound = 7,
    /// The supplied batch exceeds `BATCH_LIMIT` addresses.
    BatchTooLarge = 8,
    /// The supplied address has not been granted the required role.
    RoleNotGranted = 9,
    /// The attester validity window is empty or reversed.
    InvalidValidityWindow = 10,
    /// The requested attester region is outside the registrar's assigned region.
    RegionMismatch = 11,
    /// The registrar's concurrent enrollment quota has been reached.
    RegionalQuotaExceeded = 12,
    /// The registrar's region cannot change while its attesters remain enrolled.
    RegionalAttestersRemain = 13,
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

/// Emitted once, when the contract is initialized.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
}

/// Emitted when an attester is added to the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterAdded {
    #[topic]
    pub attester: Address,
}

/// Emitted when an already-allowlisted attester's metadata is updated via
/// `update_attester_info`. Distinguishable from `AttesterAdded`, which is
/// only emitted on initial enrollment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterInfoUpdated {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester is removed from the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRemoved {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester is suspended.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterSuspended {
    #[topic]
    pub attester: Address,
}

/// Emitted when a suspended attester is reinstated.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterReinstated {
    #[topic]
    pub attester: Address,
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
}

/// Emitted when state-changing operations are unpaused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Unpaused {
    #[topic]
    pub by: Address,
}

/// The attester registry contract.
#[contract]
pub struct AttesterRegistry;

#[contractimpl]
impl AttesterRegistry {
    /// Set the admin address authorized to manage the allowlist. Can only
    /// be called once; the caller must authorize as the given `admin`.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
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
        region: Symbol,
        quota: u32,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
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
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
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

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);

        AdminTransferred {
            previous_admin,
            new_admin: pending_admin,
        }
        .publish(&env);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Pause the contract, blocking `add_attester`, `add_attester_with_info`,
    /// `update_attester_info`, `remove_attester`, `suspend_attester`, and
    /// `reinstate_attester` until `unpause` is called. Requires a Guardian.
    pub fn pause(env: Env, guardian: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Guardian, &guardian)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: guardian }.publish(&env);
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
        Unpaused { by: admin }.publish(&env);
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
    /// `attester` is not already present (see `set_max_attesters`).
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
            .has(&DataKey::Attester(attester.clone()));
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
            if let Some(assignment) = regional_scope.as_ref() {
                if existing
                    .region
                    .as_ref()
                    .is_some_and(|current| current != &assignment.region)
                {
                    return Err(Error::RegionMismatch);
                }
            } else if existing.region != region {
                Self::clear_regional_attester(&env, &attester);
            }
        }
        let info = StoredAttesterInfo {
            license_hash: None,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        if !already_present && regional_scope.is_some() {
            Self::record_regional_attester(&env, &registrar, &attester);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Add `attester` with optional metadata to the allowlist. Requires a global
    /// or regional registrar. Regional enrollments inherit the assigned region.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`).
    pub fn add_attester_with_info(
        env: Env,
        registrar: Address,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;
        Self::validate_validity_window(valid_from, valid_until)?;
        let region = Self::scoped_region(regional_scope.as_ref(), region)?;
        let already_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
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
            if let Some(assignment) = regional_scope.as_ref() {
                if existing
                    .region
                    .as_ref()
                    .is_some_and(|current| current != &assignment.region)
                {
                    return Err(Error::RegionMismatch);
                }
            } else if existing.region != region {
                Self::clear_regional_attester(&env, &attester);
            }
        }
        let info = StoredAttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::set_validity(&env, &attester, valid_from, valid_until);
        if !already_present && regional_scope.is_some() {
            Self::record_regional_attester(&env, &registrar, &attester);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
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
        region: Option<Symbol>,
        valid_from: Option<u64>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        Self::validate_validity_window(valid_from, valid_until)?;
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()))
        {
            return Err(Error::AttesterNotFound);
        }
        let existing: StoredAttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))
            .ok_or(Error::AttesterNotFound)?;
        if existing.region != region {
            Self::clear_regional_attester(&env, &attester);
        }
        let info = StoredAttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::set_validity(&env, &attester, valid_from, valid_until);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterInfoUpdated { attester }.publish(&env);
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
            if !env.storage().persistent().has(&key) {
                Self::ensure_regional_quota(&env, &registrar, regional_scope.as_ref())?;
                if count >= max {
                    return Err(Error::AllowlistFull);
                }
                let info = StoredAttesterInfo {
                    license_hash: None,
                    region: regional_scope
                        .as_ref()
                        .map(|assignment| assignment.region.clone()),
                };
                env.storage().persistent().set(&key, &info);
                if regional_scope.is_some() {
                    Self::record_regional_attester(&env, &registrar, &attester);
                }
                count += 1;
                AttesterAdded {
                    attester: attester.clone(),
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
        }) && attesters.len() > REGIONAL_BATCH_LIMIT
        {
            return Err(Error::BatchTooLarge);
        }

        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
                env.storage()
                    .persistent()
                    .remove(&DataKey::Suspended(attester.clone()));
                env.storage()
                    .persistent()
                    .remove(&DataKey::AttesterValidity(attester.clone()));
                Self::clear_regional_attester(&env, &attester);
                if count > 0 {
                    count -= 1;
                }
                AttesterRemoved {
                    attester: attester.clone(),
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
        let was_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Attester(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::AttesterValidity(attester.clone()));
        Self::clear_regional_attester(&env, &attester);
        if was_present {
            let count = Self::attester_count(&env);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::AttesterCount, &(count - 1));
            }
        }
        AttesterRemoved { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Set the soft cap on the number of allowlisted attesters. Requires the
    /// admin's authorization. Does not evict existing attesters if lowered
    /// below the current count; it only blocks further `add_attester` calls.
    pub fn set_max_attesters(env: Env, max_attesters: u32) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
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
    /// **Note:** this function does **not** check whether `attester` was ever
    /// added via `add_attester`. If called on an address that is not in the
    /// allowlist, it silently sets the `Suspended` storage key and emits
    /// `AttesterSuspended` for that address — a no-op from an access-control
    /// perspective because `is_attester` also checks for an `Attester` storage
    /// entry, so the phantom suspension has no effect on allowlist queries.
    /// This diverges from `update_attester_info`, which returns
    /// `Error::AttesterNotFound` for unknown addresses. The inconsistency is
    /// known and documented here rather than silently changed; a follow-up
    /// issue should decide whether to align both functions.
    pub fn suspend_attester(env: Env, registrar: Address, attester: Address) -> Result<(), Error> {
        let regional_scope = Self::registrar_scope(&env, &registrar)?;
        Self::require_not_paused(&env)?;
        if let Some(assignment) = regional_scope {
            let info = Self::attester_info(&env, &attester).ok_or(Error::AttesterNotFound)?;
            if info.region.as_ref() != Some(&assignment.region) {
                return Err(Error::RegionMismatch);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::Suspended(attester.clone()), &true);
        AttesterSuspended { attester }.publish(&env);
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
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        AttesterReinstated { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Whether `attester` is currently allowlisted (and not suspended). Callable by anyone,
    /// including other contracts (e.g. `attestation-registry`).
    pub fn is_attester(env: Env, attester: Address) -> bool {
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()))
        {
            return false;
        }
        !env.storage()
            .persistent()
            .has(&DataKey::Suspended(attester.clone()))
            && Self::valid_at(Self::validity(&env, &attester), env.ledger().timestamp())
    }

    /// Get the optional metadata associated with `attester` if they are allowlisted.
    pub fn get_attester_info(env: Env, attester: Address) -> Option<AttesterInfo> {
        Self::attester_info(&env, &attester)
    }

    /// Get `attester`'s metadata together with its current suspension state
    /// in a single call. Returns `None` if `attester` is not currently
    /// allowlisted (never added, or since removed).
    pub fn get_attester_status(env: Env, attester: Address) -> Option<AttesterStatus> {
        let info = Self::attester_info(&env, &attester)?;
        let suspended = env
            .storage()
            .persistent()
            .has(&DataKey::Suspended(attester));
        let status = Self::status(&info, suspended, env.ledger().timestamp());
        Some(AttesterStatus {
            info,
            suspended,
            status,
        })
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
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        Upgraded { new_wasm_hash }.publish(&env);
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
        region: Option<Symbol>,
    ) -> Result<Option<Symbol>, Error> {
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
        let (valid_from, valid_until) = Self::validity(env, attester);
        Some(AttesterInfo {
            license_hash: stored.license_hash,
            region: stored.region,
            valid_from,
            valid_until,
        })
    }

    fn valid_at(validity: (Option<u64>, Option<u64>), timestamp: u64) -> bool {
        let (valid_from, valid_until) = validity;
        valid_from.is_none_or(|start| timestamp >= start)
            && valid_until.is_none_or(|end| timestamp < end)
    }

    fn status(info: &AttesterInfo, suspended: bool, timestamp: u64) -> AttesterStatusKind {
        if suspended {
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod large_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
