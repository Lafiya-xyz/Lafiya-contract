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

/// Storage keys for the attestation registry.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to (re)point `AttesterRegistry`.
    Admin,
    /// The deployed `attester-registry` contract consulted on every `attest` call.
    AttesterRegistry,
    /// Attestation for a given record hash at a specific sequence number.
    Attestation(BytesN<32>, u64),
    /// Latest sequence number for a given record hash.
    AttestationSequence(BytesN<32>),
    /// Count of attestations for a given record hash (for bounded history).
    AttestationCount(BytesN<32>),
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
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
        };

        // Get and increment the sequence number for this record hash
        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .unwrap_or(0);
        let new_sequence = sequence + 1;

        // Store the attestation with the new sequence number
        env.storage()
            .persistent()
            .set(&DataKey::Attestation(record_hash.clone(), new_sequence), &attestation);

        // Update the sequence number
        env.storage()
            .persistent()
            .set(&DataKey::AttestationSequence(record_hash.clone()), &new_sequence);

        // Update the count and enforce bounded history
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let new_count = count + 1;

        if new_count > MAX_HISTORY {
            // Remove the oldest attestation (FIFO eviction)
            let oldest_sequence = new_count.saturating_sub(MAX_HISTORY);
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), oldest_sequence));
        }

        env.storage()
            .persistent()
            .set(&DataKey::AttestationCount(record_hash.clone()), &new_count);

        AttestationRecorded {
            record_hash,
            attester,
            timestamp: attestation.timestamp,
        }
        .publish(&env);

        Ok(attestation)
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
