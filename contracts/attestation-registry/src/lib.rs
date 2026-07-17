#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address,
    BytesN, Env,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
}

/// Storage keys for the attestation registry.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to (re)point `AttesterRegistry`.
    Admin,
    /// The deployed `attester-registry` contract consulted on every `attest` call.
    AttesterRegistry,
    /// Latest attestation recorded for a given record hash.
    Attestation(BytesN<32>),
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// A single attestation: proof that `attester` verified the off-chain
/// record whose hash is the lookup key, at `timestamp`. Never contains the
/// underlying health data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    pub attester: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRecorded {
    #[topic]
    pub record_hash: BytesN<32>,
    pub attester: Address,
    pub timestamp: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    AttesterNotAllowlisted = 3,
}

#[contract]
pub struct AttestationRegistry;

#[contractimpl]
impl AttestationRegistry {
    /// Set the admin and the `attester-registry` contract this registry
    /// consults for allowlist checks. Can only be called once; the caller
    /// must authorize as the given `admin`.
    pub fn initialize(env: Env, admin: Address, attester_registry: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Record that `attester` verified the record hashing to `record_hash`.
    /// Requires `attester`'s authorization and that `attester` is
    /// currently allowlisted in the configured `attester-registry`.
    /// Overwrites any prior attestation for the same `record_hash`.
    pub fn attest(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();

        let registry_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

        let attestation = Attestation {
            attester,
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attestation(record_hash.clone()), &attestation);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AttestationRecorded {
            record_hash,
            attester: attestation.attester.clone(),
            timestamp: attestation.timestamp,
        }
        .publish(&env);

        Ok(attestation)
    }

    /// Look up the latest attestation for `record_hash`, if any. Callable
    /// by anyone — this is what lets a responder's QR scan independently
    /// check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        env.storage()
            .persistent()
            .get(&DataKey::Attestation(record_hash))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
