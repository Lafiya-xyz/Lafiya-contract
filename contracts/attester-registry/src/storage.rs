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
            DataKey::Admin
            | DataKey::EventSeq
            | DataKey::PendingAdmin
            | DataKey::SchemaVersion
            | DataKey::Paused
            | DataKey::MaxAttesters
            | DataKey::AttesterCount
            | DataKey::PendingAdminExpiresAt
            | DataKey::Role(..)
            | DataKey::RegionalRegistrar(..) => Durability::Instance,
            DataKey::Attester(..)
            | DataKey::Suspended(..)
            | DataKey::AttesterRotation(..)
            | DataKey::AttesterStatusChange(..)
            | DataKey::AttesterStatusChangeCount(..)
            | DataKey::AttesterValidity(..)
            | DataKey::RegionalRegistrarCount(..)
            | DataKey::RegionalAttester(..) => Durability::Persistent,
        }
    }
}

/// Returns the next per-contract event sequence number (starting at 1) and
/// persists it. Called while building each event, so the counter advances in
/// the same transaction as the emission and rolls back with it.
pub fn next_event_seq(env: &Env) -> u64 {
    let seq: u64 = get(env, &DataKey::EventSeq).unwrap_or(0) + 1;
    set(env, &DataKey::EventSeq, &seq);
    seq
}

/// Full post-state snapshot of a single attester, as carried by the
/// `AttesterStateChanged` lifecycle event. Built from storage in one place so
/// every mutating path emits a consistent view and indexers can rebuild the
/// attesters table without storage reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttesterSnapshot {
    /// Current lifecycle status code (see `AttesterStatus`).
    pub status: u32,
    /// Tier code assigned to the attester.
    pub tier: u32,
    /// Region code the attester is registered in.
    pub region: u32,
    /// Whether the attester is currently on probation.
    pub probation: bool,
    /// Hash of the off-chain metadata document (zero when unset).
    pub info_hash: soroban_sdk::BytesN<32>,
    /// Sequence number of the event that carries this snapshot.
    pub seq: u64,
}

/// Reads the current post-state for `attester` and stamps it with the next
/// event sequence number. Call this after applying a mutation and before
/// emitting `AttesterStateChanged` so the snapshot and the event agree.
pub fn attester_snapshot(env: &Env, attester: &soroban_sdk::Address) -> AttesterSnapshot {
    let record: Option<crate::AttesterRecord> = get(env, &DataKey::Attester(attester.clone()));
    let suspended: bool = has(env, &DataKey::Suspended(attester.clone()));
    let record = record.unwrap_or_else(|| crate::AttesterRecord::empty(env));
    AttesterSnapshot {
        status: record.status,
        tier: record.tier,
        region: record.region,
        probation: record.probation || suspended,
        info_hash: record.info_hash,
        seq: next_event_seq(env),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{AttesterRegistry, Role};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    /// One sample of every `DataKey` variant. Keep in sync with the enum.
    fn all_keys(env: &Env) -> std::vec::Vec<DataKey> {
        std::vec![
            DataKey::Admin,
            DataKey::EventSeq,
            DataKey::PendingAdmin,
            DataKey::Attester(Address::generate(env)),
            DataKey::Suspended(Address::generate(env)),
            DataKey::SchemaVersion,
            DataKey::Paused,
            DataKey::MaxAttesters,
            DataKey::AttesterCount,
            DataKey::PendingAdminExpiresAt,
            DataKey::AttesterRotation(Address::generate(env)),
            DataKey::AttesterStatusChange(Address::generate(env), 1),
            DataKey::AttesterStatusChangeCount(Address::generate(env)),
            DataKey::Role(Role::Registrar, Address::generate(env)),
            DataKey::AttesterValidity(Address::generate(env)),
            DataKey::RegionalRegistrar(Address::generate(env)),
            DataKey::RegionalRegistrarCount(Address::generate(env)),
            DataKey::RegionalAttester(Address::generate(env)),
        ]
    }

    #[test]
    fn every_data_key_maps_to_exactly_one_durability_class() {
        let env = lafiya_test_utils::new_env();
        let id = env.register(AttesterRegistry, (Address::generate(&env),));
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
