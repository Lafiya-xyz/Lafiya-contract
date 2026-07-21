#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address,
    BytesN, Env, Vec,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
}

/// Maximum number of historical attestations to keep per record hash.
/// This bounds storage growth per re-attestation. When exceeded,
/// the oldest attestation is removed (FIFO eviction).
const MAX_HISTORY: u64 = 10;

const SCHEMA_VERSION: u32 = 1;

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Storage keys for the attestation registry.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to (re)point `AttesterRegistry`.
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
}

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
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRecorded {
    #[topic]
    pub record_hash: BytesN<32>,
    pub attester: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRevoked {
    #[topic]
    pub record_hash: BytesN<32>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    AttesterNotAllowlisted = 3,
    NoPendingTransfer = 4,
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
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(())
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Return the configured attester-registry contract address.
    pub fn get_attester_registry(env: Env) -> Result<Address, Error> {
        Self::attester_registry(&env)
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

    /// Record that `attester` verified the record hashing to `record_hash`.
    /// Requires `attester`'s authorization and that `attester` is
    /// currently allowlisted in the configured `attester-registry`.
    /// Stores the attestation with an incrementing sequence number,
    /// maintaining a bounded history (MAX_HISTORY entries per hash).
    pub fn attest(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();

        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

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
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AttestationRecorded {
            record_hash,
            attester,
            timestamp: attestation.timestamp,
        }
        .publish(&env);

        Ok(attestation)
    }

    /// Revoke all attestations for `record_hash`. Gated by admin authorization.
    pub fn revoke_attestation(env: Env, record_hash: BytesN<32>) -> Result<(), Error> {
        let admin: Address = Self::admin(&env)?;
        admin.require_auth();

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .ok_or(Error::NotInitialized)?;

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

        for seq in start_sequence..=sequence {
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), seq));
        }
        env.storage()
            .persistent()
            .remove(&DataKey::AttestationSequence(record_hash.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::AttestationCount(record_hash.clone()));

        AttestationRevoked { record_hash }.publish(&env);

        Ok(())
    }

    /// Look up the latest attestation for `record_hash`, if any. Callable
    /// by anyone — this is what lets a responder's QR scan independently
    /// check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))?;
        env.storage()
            .persistent()
            .get(&DataKey::Attestation(record_hash, sequence))
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

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn attester_registry(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
