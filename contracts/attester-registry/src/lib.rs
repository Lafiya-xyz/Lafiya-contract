#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    Symbol,
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
}

/// Metadata associated with an allowlisted attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterInfo {
    pub license_hash: Option<BytesN<32>>,
    pub region: Option<Symbol>,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    NoPendingTransfer = 3,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterAdded {
    #[topic]
    pub attester: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRemoved {
    #[topic]
    pub attester: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterSuspended {
    #[topic]
    pub attester: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterReinstated {
    #[topic]
    pub attester: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
}

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

        Ok(())
    }

    /// Add `attester` to the allowlist. Requires the admin's authorization.
    pub fn add_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let info = AttesterInfo {
            license_hash: None,
            region: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
    pub fn add_attester_with_info(
        env: Env,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let info = AttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Remove `attester` from the allowlist. Requires the admin's
    /// authorization. A no-op if the attester was never allowlisted.
    pub fn remove_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::Attester(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        AttesterRemoved { attester }.publish(&env);
        Ok(())
    }

    /// Suspend an allowlisted attester. Requires the admin's authorization.
    pub fn suspend_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Suspended(attester.clone()), &true);
        AttesterSuspended { attester }.publish(&env);
        Ok(())
    }

    /// Reinstate a suspended attester. Requires the admin's authorization.
    pub fn reinstate_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        AttesterReinstated { attester }.publish(&env);
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
        Ok(())
    }

    /// The storage schema version recorded for this instance. `0` means no
    /// version has been recorded: the instance was deployed before schema
    /// versioning landed (legacy) or was never initialized.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(0)
    }

    /// Replace this contract's code with the wasm blob identified by
    /// `new_wasm_hash`. The blob must already have been uploaded to the
    /// ledger (e.g. `stellar contract upload`); otherwise the ledger rejects
    /// the update. Requires the admin's authorization. Instance and
    /// persistent storage are untouched — the new code starts exactly where
    /// the old code left off. The swap itself takes effect once this
    /// invocation finishes successfully. See
    /// `docs/runbooks/contract-upgrade.md` for the full production
    /// procedure, including how reviewers verify `new_wasm_hash` against
    /// the audited source before this call is signed.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
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
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

#[cfg(test)]
mod large_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
