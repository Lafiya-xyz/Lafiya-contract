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
            | DataKey::AttesterRegistry
            | DataKey::SchemaVersion
            | DataKey::Paused
            | DataKey::PendingAdminExpiresAt
            | DataKey::RateLimit
            | DataKey::Role(..)
            | DataKey::MaxAttestationAge => Durability::Instance,
            DataKey::Attestation(..)
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
            | DataKey::NextRecordHash(..) => Durability::Persistent,
            DataKey::RateWindow(..) => Durability::Temporary,
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
