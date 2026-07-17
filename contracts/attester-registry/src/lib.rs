#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env,
};

/// Storage keys for the attester registry.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to add/remove attesters.
    Admin,
    /// Presence of this key (mapped to `true`) means the address is an
    /// allowlisted attester.
    Attester(Address),
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
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Add `attester` to the allowlist. Requires the admin's authorization.
    pub fn add_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
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
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterRemoved { attester }.publish(&env);
        Ok(())
    }

    /// Whether `attester` is currently allowlisted. Callable by anyone,
    /// including other contracts (e.g. `attestation-registry`).
    pub fn is_attester(env: Env, attester: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Attester(attester))
            .unwrap_or(false)
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
