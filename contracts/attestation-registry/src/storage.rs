//! Typed storage access layer for this contract (see
//! `lafiya_contract_common::storage`). Every `DataKey` variant is bound to
//! exactly one durability class here; the `match` is exhaustive, so adding a
//! variant without classifying it does not compile.
//!
//! Extension steps: add the variant to `DataKey` (append only), add it to the
//! `durability` match below, and add a sample value to `all_keys` in the
//! tests. All storage access in `lib.rs` must go through this module.

use crate::DataKey;
pub use lafiya_contract_common::storage::{
    extend_instance_ttl, extend_ttl, get, has, remove, set, Durability, StorageKey,
};
use soroban_sdk::Env;

impl StorageKey for DataKey {
    fn durability(&self) -> Durability {
        match self {
            // Only keys that are never written on a hot path stay in the
            // instance entry: admin/role/config changes are rare, and every
            // instance write rewrites the whole entry (serializing parallel
            // transactions), so hot paths must not touch these.
            DataKey::Admin
            | DataKey::PendingAdmin
            | DataKey::AttesterRegistry
            | DataKey::SchemaVersion
            | DataKey::Paused
            | DataKey::PendingAdminExpiresAt
            | DataKey::MaxAttestationAge => Durability::Instance,
            // Rarely changing config moved out of the instance entry so that
            // attest/co-attest/claim/add_attester never rewrite it.
            DataKey::RateLimit
            | DataKey::Role(..)
            | DataKey::EventSeq
            | DataKey::Attestation(..)
            | DataKey::AttestationSequence(..)
            | DataKey::AttestationCount(..)
            | DataKey::RateLimitOverride(..)
            | DataKey::PatientConsent(..)
            | DataKey::PatientConsentValidity(..)
            | DataKey::AttestationExpiry(..)
            | DataKey::AttestationBatch(..)
            | DataKey::AttestationWithdrawn(..)
            | DataKey::RecordRevoked(..)
            | DataKey::RevokedThroughSequence(..)
            | DataKey::PreviousRecordHash(..)
            | DataKey::NextRecordHash(..)
            | DataKey::StatusListSegment(..)
            | DataKey::StatusListRoot(..)
            | DataKey::StatusListEpochSegments(..)
            | DataKey::StatusListEpochRoot(..)
            | DataKey::StatusListEpochBits(..)
            | DataKey::StatusListEpochRetention(..) => Durability::Persistent,
            DataKey::RateWindow(..) => Durability::Temporary,
        }
    }
}

/// Returns the next per-contract event sequence number (starting at 1) and
/// persists it.
///
/// NOTE: this counter is no longer written on hot paths. It is retained only
/// for legacy/administrative events; hot-path events carry their ordering key
/// from the ledger (`ledger sequence`, `tx index`, `event index`) supplied by
/// RPC, so no shared entry is written. See `event_ordering_key`.
pub fn next_event_seq(env: &Env) -> u64 {
    let seq: u64 = get(env, &DataKey::EventSeq).unwrap_or(0) + 1;
    set(env, &DataKey::EventSeq, &seq);
    seq
}

/// Ordering key for events emitted on hot paths, derived from ledger data
/// instead of a shared counter. `(ledger, tx_index, event_index)` is globally
/// unique and monotonic per ledger, so indexers can order events and detect
/// gaps without any on-chain shared write.
pub fn event_ordering_key(env: &Env, tx_index: u32, event_index: u32) -> (u32, u32, u32) {
    (env.ledger().sequence(), tx_index, event_index)
}

/// Per-record sequence number, scoped to a single record so that concurrent
/// attestations of different records never contend on one entry.
pub fn next_record_seq(env: &Env, record_hash: &soroban_sdk::BytesN<32>) -> u64 {
    let key = DataKey::AttestationSequence(record_hash.clone());
    let seq: u64 = get(env, &key).unwrap_or(0) + 1;
    set(env, &key, &seq);
    seq
}

/// Per-attester sequence number, scoped to a single attester so that
/// concurrent attestations by different attesters never contend.
pub fn next_attester_seq(env: &Env, attester: &soroban_sdk::Address) -> u64 {
    let key = DataKey::AttesterSequence(attester.clone());
    let seq: u64 = get(env, &key).unwrap_or(0) + 1;
    set(env, &key, &seq);
    seq
}

/// Publishes a status-list segment for `(epoch, index)` together with its
/// hash. Segments are epoch-scoped so a verifier can read `root(epoch)` first
/// and then only segments belonging to that epoch.
pub fn put_status_list_segment(
    env: &Env,
    epoch: u64,
    index: u32,
    bits: &soroban_sdk::Bytes,
    segment_hash: &soroban_sdk::BytesN<32>,
) {
    set(
        env,
        &DataKey::StatusListSegment(epoch, index),
        &(bits.clone(), segment_hash.clone()),
    );
    let mut hashes: soroban_sdk::Vec<soroban_sdk::BytesN<32>> =
        get(env, &DataKey::StatusListEpochSegments(epoch)).unwrap_or(soroban_sdk::Vec::new(env));
    hashes.push_back(segment_hash.clone());
    set(env, &DataKey::StatusListEpochSegments(epoch), &hashes);
}

/// Reads a status-list segment for `(epoch, index)`.
pub fn get_status_list_segment(
    env: &Env,
    epoch: u64,
    index: u32,
) -> Option<(soroban_sdk::Bytes, soroban_sdk::BytesN<32>)> {
    get(env, &DataKey::StatusListSegment(epoch, index))
}

/// Returns the segment hashes published for `epoch`.
pub fn status_list_epoch_segments(
    env: &Env,
    epoch: u64,
) -> soroban_sdk::Vec<soroban_sdk::BytesN<32>> {
    get(env, &DataKey::StatusListEpochSegments(epoch)).unwrap_or(soroban_sdk::Vec::new(env))
}

/// Activates `root` for `epoch`. Callers must have verified that every
/// segment referenced by the root is present for the epoch and that the Merkle
/// root over the segment hashes recomputes on-chain.
pub fn set_status_list_root(env: &Env, epoch: u64, root: &soroban_sdk::BytesN<32>) {
    set(env, &DataKey::StatusListRoot(epoch), root);
    set(env, &DataKey::StatusListEpochRoot(epoch), root);
}

/// Returns the active root for `epoch`, if any.
pub fn get_status_list_root(env: &Env, epoch: u64) -> Option<soroban_sdk::BytesN<32>> {
    get(env, &DataKey::StatusListRoot(epoch))
}

/// Records the revocation bits of `epoch` so the next epoch can enforce
/// monotonic revocation (a set bit may never be cleared without an explicit
/// unrevoke).
pub fn set_status_list_epoch_bits(env: &Env, epoch: u64, bits: &soroban_sdk::Bytes) {
    set(env, &DataKey::StatusListEpochBits(epoch), bits);
}

/// Returns the revocation bits recorded for `epoch`, if any.
pub fn get_status_list_epoch_bits(env: &Env, epoch: u64) -> Option<soroban_sdk::Bytes> {
    get(env, &DataKey::StatusListEpochBits(epoch))
}

/// Sets the ledger at which `epoch` may be pruned. Old epochs stay readable
/// until this point so in-flight verifications are not broken.
pub fn set_status_list_epoch_retention(env: &Env, epoch: u64, expires_at: u32) {
    set(env, &DataKey::StatusListEpochRetention(epoch), &expires_at);
}

/// Returns the retention deadline of `epoch`, if set.
pub fn get_status_list_epoch_retention(env: &Env, epoch: u64) -> Option<u32> {
    get(env, &DataKey::StatusListEpochRetention(epoch))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{AttestationRegistry, Role};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::BytesN;
    use soroban_sdk::{Address, Env};

    /// One sample of every `DataKey` variant. Keep in sync with the enum.
    fn all_keys(env: &Env) -> std::vec::Vec<DataKey> {
        std::vec![
            DataKey::Admin,
            DataKey::EventSeq,
            DataKey::AttesterSequence(Address::generate(env)),
            DataKey::StatusListSegment(1, 0),
            DataKey::StatusListRoot(1),
            DataKey::StatusListEpochSegments(1),
            DataKey::StatusListEpochRoot(1),
            DataKey::StatusListEpochBits(1),
            DataKey::StatusListEpochRetention(1),
            DataKey::PendingAdmin,
            DataKey::AttesterRegistry,
            DataKey::Attestation(BytesN::from_array(env, &[7; 32]), 1),
            DataKey::AttestationSequence(BytesN::from_array(env, &[7; 32])),
            DataKey::AttestationCount(BytesN::from_array(env, &[7; 32])),
            DataKey::SchemaVersion,
            DataKey::Paused,
            DataKey::PendingAdminExpiresAt,
            DataKey::RateLimit,
            DataKey::RateLimitOverride(Address::generate(env)),
            DataKey::RateWindow(Address::generate(env)),
            DataKey::Role(Role::Guardian, Address::generate(env)),
            DataKey::MaxAttestationAge,
            DataKey::PatientConsent(
                BytesN::from_array(env, &[7; 32]),
                Address::generate(env),
                Address::generate(env)
            ),
            DataKey::PatientConsentValidity(
                BytesN::from_array(env, &[7; 32]),
                Address::generate(env),
                Address::generate(env)
            ),
            DataKey::AttestationExpiry(BytesN::from_array(env, &[7; 32]), 1),
            DataKey::AttestationBatch(BytesN::from_array(env, &[7; 32])),
            DataKey::AttestationWithdrawn(BytesN::from_array(env, &[7; 32]), 1),
            DataKey::RecordRevoked(BytesN::from_array(env, &[7; 32])),
            DataKey::RevokedThroughSequence(BytesN::from_array(env, &[7; 32])),
            DataKey::PreviousRecordHash(BytesN::from_array(env, &[7; 32])),
            DataKey::NextRecordHash(BytesN::from_array(env, &[7; 32])),
        ]
    }

    #[test]
    fn every_data_key_maps_to_exactly_one_durability_class() {
        let env = lafiya_test_utils::new_env();
        let admin = Address::generate(&env);
        let registry = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
        let id = env.register(AttestationRegistry, (admin, registry));
        let keys = all_keys(&env);
        // Every variant in the enum is covered by exactly one sample.
        let variants = include_str!("lib.rs")
            .split("enum DataKey {")
            .nth(1)
            .and_then(|s| s.split("\n}").next())
            .map(|s| {
                s.lines()
                    .filter(|l| {
                        l.starts_with("    ")
                            && !l.starts_with("     ")
                            && !l.trim_start().starts_with("//")
                            && !l.trim().is_empty()
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(
            keys.len(),
            variants,
            "all_keys must list every DataKey variant"
        );

        env.as_contract(&id, || {
            for (i, key) in keys.iter().enumerate() {
                set(&env, key, &1u32);
                let s = env.storage();
                let present = (
                    s.instance().has(key),
                    s.persistent().has(key),
                    s.temporary().has(key),
                );
                let expected = match key.durability() {
                    Durability::Instance => (true, false, false),
                    Durability::Persistent => (false, true, false),
                    Durability::Temporary => (false, false, true),
                };
                assert_eq!(present, expected, "wrong store for key #{}", i);
                remove(&env, key);
                assert!(!has(&env, key));
            }
        });
    }

    #[test]
    fn no_direct_storage_calls_outside_storage_module() {
        assert!(
            !include_str!("lib.rs").contains("storage()"),
            "lib.rs must use the typed accessors in storage.rs"
        );
    }
}
