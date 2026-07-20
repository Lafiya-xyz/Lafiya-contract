#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    Symbol,
};

const SCHEMA_VERSION: u32 = 1;

/// Storage keys for the attester registry.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to add/remove attesters.
    Admin,
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// Presence of this key (mapped to `true`) means the address is an
    /// allowlisted attester.
    Attester(Address),
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
        AttesterRemoved { attester }.publish(&env);
        Ok(())
    }

    /// Whether `attester` is currently allowlisted. Callable by anyone,
    /// including other contracts (e.g. `attestation-registry`).
    pub fn is_attester(env: Env, attester: Address) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::Attester(attester))
    }

    /// Get the optional metadata associated with `attester` if they are allowlisted.
    pub fn get_attester_info(env: Env, attester: Address) -> Option<AttesterInfo> {
        env.storage()
            .persistent()
            .get(&DataKey::Attester(attester))
    }

    /// Query the current storage schema version of the contract.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(1)
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
#[cfg(test)]
mod large_test;
