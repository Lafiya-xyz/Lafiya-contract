#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env, Vec,
};

/// Maximum number of addresses that may be processed in a single batch
/// add_attesters / remove_attesters call.
///
/// Rationale: benchmarks (see `test_batch_budget`) show that a batch of 50
/// addresses consumes well under the 100 M-instruction per-transaction cap
/// even on a cold ledger (all keys missing).  Batches beyond this size are
/// rejected with `Error::BatchTooLarge` to give an early, deterministic error
/// rather than a silent resource-limit abort.
pub const BATCH_LIMIT: u32 = 40;

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

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    /// The supplied batch exceeds `BATCH_LIMIT` addresses.
    BatchTooLarge = 3,
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
        Ok(())
    }

    /// Add `attester` to the allowlist. Requires the admin's authorization.
    pub fn add_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &true);
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

    /// Add multiple attesters to the allowlist in a single transaction.
    ///
    /// Requires the admin's authorization. Returns `Error::BatchTooLarge` if
    /// `attesters.len() > BATCH_LIMIT`. Addresses that are already
    /// allowlisted are silently skipped (idempotent), so the call never fails
    /// due to duplicates in the batch and no duplicate events are emitted.
    /// Exactly one `AttesterAdded` event is emitted per newly added address.
    pub fn add_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            // Skip if already allowlisted — no storage write, no event.
            if !env.storage().persistent().has(&key) {
                env.storage().persistent().set(&key, &true);
                AttesterAdded { attester }.publish(&env);
            }
        }
        Ok(())
    }

    /// Remove multiple attesters from the allowlist in a single transaction.
    ///
    /// Requires the admin's authorization. Returns `Error::BatchTooLarge` if
    /// `attesters.len() > BATCH_LIMIT`. Addresses that are not currently
    /// allowlisted are silently skipped (idempotent), so the call never fails
    /// if an address was already removed and no spurious events are emitted.
    /// Exactly one `AttesterRemoved` event is emitted per address that was
    /// actually removed.
    pub fn remove_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            // Skip if not allowlisted — no storage remove, no event.
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
                AttesterRemoved { attester }.publish(&env);
            }
        }
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
