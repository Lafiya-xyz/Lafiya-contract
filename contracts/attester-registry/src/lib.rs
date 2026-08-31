//! Soroban contract maintaining the allowlist of attesters authorized to
//! call `attest` on the `attestation-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    Symbol, Vec,
};

const SCHEMA_VERSION: u32 = 1;

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
}

/// Metadata associated with an allowlisted attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterInfo {
    /// Hash of the attester's off-chain license/credential document, if any.
    pub license_hash: Option<BytesN<32>>,
    /// The geographic region the attester is authorized to attest for, if any.
    pub region: Option<Symbol>,
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

/// Errors returned by the attester registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// `accept_admin` was called with no pending admin transfer.
    NoPendingTransfer = 3,
    /// The requested operation is blocked while the contract is paused.
    ContractPaused = 4,
    /// The allowlist is at its configured maximum size.
    AllowlistFull = 5,
    /// A storage migration was invoked but the contract is already current.
    MigrationNotRequired = 6,
    /// The referenced attester is not currently allowlisted (never added,
    /// or since removed).
    AttesterNotFound = 7,
    /// The supplied batch exceeds `BATCH_LIMIT` addresses.
    BatchTooLarge = 8,
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

    /// Propose a new admin address. The caller must authorize as the current admin.
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
    /// `reinstate_attester` until `unpause` is called. Requires the admin's
    /// authorization.
    pub fn pause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: admin }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the admin's authorization.
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

    /// Add `attester` to the allowlist. Requires the admin's authorization.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`).
    pub fn add_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        let already_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
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
        let info = AttesterInfo {
            license_hash: None,
            region: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`).
    pub fn add_attester_with_info(
        env: Env,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        let already_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
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
        let info = AttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Update the metadata of an already-allowlisted `attester`. Requires
    /// the admin's authorization. Unlike `add_attester_with_info`, this
    /// never enrolls a new attester: it fails with `Error::AttesterNotFound`
    /// if `attester` is not currently allowlisted (never added, or since
    /// removed), and always emits `AttesterInfoUpdated` rather than
    /// `AttesterAdded`, so profile changes are distinguishable from
    /// enrollment.
    pub fn update_attester_info(
        env: Env,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()))
        {
            return Err(Error::AttesterNotFound);
        }
        let info = AttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterInfoUpdated { attester }.publish(&env);
        Ok(())
    }

    /// Add multiple attesters to the allowlist in a single transaction.
    ///
    /// Requires the admin's authorization. Blocked while the contract is paused.
    /// Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
    /// Returns `Error::AllowlistFull` if adding the new (non-duplicate)
    /// addresses would exceed the configured `max_attesters` cap. Addresses
    /// that are already allowlisted are silently skipped (idempotent), so the
    /// call never fails due to duplicates in the batch and no duplicate events
    /// are emitted. Exactly one `AttesterAdded` event is emitted per newly
    /// added address.
    pub fn add_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        let max = Self::max_attesters(&env);
        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            if !env.storage().persistent().has(&key) {
                if count >= max {
                    return Err(Error::AllowlistFull);
                }
                let info = AttesterInfo {
                    license_hash: None,
                    region: None,
                };
                env.storage().persistent().set(&key, &info);
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
    /// Requires the admin's authorization. Blocked while the contract is paused.
    /// Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
    /// Addresses that are not currently allowlisted are silently skipped
    /// (idempotent), so the call never fails if an address was already removed
    /// and no spurious events are emitted. Exactly one `AttesterRemoved` event
    /// is emitted per address that was actually removed.
    pub fn remove_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
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

    /// Remove `attester` from the allowlist. Requires the admin's
    /// authorization. A no-op if the attester was never allowlisted.
    pub fn remove_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
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

    /// Suspend an allowlisted attester. Requires the admin's authorization.
    pub fn suspend_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::Suspended(attester.clone()), &true);
        AttesterSuspended { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Reinstate a suspended attester. Requires the admin's authorization.
    pub fn reinstate_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
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
            .has(&DataKey::Suspended(attester))
    }

    /// Get the optional metadata associated with `attester` if they are allowlisted.
    pub fn get_attester_info(env: Env, attester: Address) -> Option<AttesterInfo> {
        env.storage().persistent().get(&DataKey::Attester(attester))
    }

    /// Get `attester`'s metadata together with its current suspension state
    /// in a single call. Returns `None` if `attester` is not currently
    /// allowlisted (never added, or since removed).
    pub fn get_attester_status(env: Env, attester: Address) -> Option<AttesterStatus> {
        let info: AttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))?;
        let suspended = env
            .storage()
            .persistent()
            .has(&DataKey::Suspended(attester));
        Some(AttesterStatus { info, suspended })
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

        // Per-version migration steps, oldest first. This build introduces
        // schema version 1, whose layout is identical to the legacy
        // (unversioned) layout, so no data reshaping is required here.
        // Schema-changing releases insert their steps below, guarded by the
        // version they migrate FROM, e.g.:
        //
        //   if stored < 2 { /* move/reshape v1 data into the v2 layout */ }
        //   if stored < 3 { /* ... */ }

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
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
