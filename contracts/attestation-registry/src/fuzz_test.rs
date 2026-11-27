//! Property-based fuzz testing for `attest`. Targets two things the issue
//! called out specifically: panics on arbitrary/adversarial `record_hash`
//! byte patterns and re-attestation behavior.
//!
//! Run just this target locally with more cases via:
//! `PROPTEST_CASES=10000 cargo test -p attestation-registry fuzz_test -- --nocapture`

extern crate std;

use super::*;
use attester_registry::{AttesterRegistry, AttesterRegistryClient};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, Vec};

fn setup(
    env: &Env,
) -> (
    AttestationRegistryClient<'_>,
    AttesterRegistryClient<'_>,
    Address,
) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let attester_registry_id = env.register(AttesterRegistry, (admin.clone(),));
    let attester_registry_client = AttesterRegistryClient::new(env, &attester_registry_id);
    attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);
    let contract_id = env.register(AttestationRegistry, (admin.clone(), attester_registry_id));
    (
        AttestationRegistryClient::new(env, &contract_id),
        attester_registry_client,
        admin,
    )
}

fn attest_with_consent(
    env: &Env,
    client: &AttestationRegistryClient<'_>,
    attester: &Address,
    record_hash: &BytesN<32>,
) -> Result<Attestation, ()> {
    let patient = Address::generate(env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, attester, record_hash, &expires_at);
    client
        .try_attest(attester, &patient, record_hash)
        .map(|result| result.expect("conversion"))
        .map_err(|_| ())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any 32-byte `record_hash` value — including all-zero, all-`0xFF`,
    /// and arbitrary bytes — must be accepted by `attest` for an
    /// allowlisted attester and readable back afterwards, with no panic.
    #[test]
    fn attest_never_panics_on_arbitrary_record_hash(bytes in proptest::array::uniform32(any::<u8>())) {
        let env = Env::default();
        let (client, attester_registry_client, admin) = setup(&env);
        let attester = Address::generate(&env);
        attester_registry_client.add_attester(&admin, &attester);

        let record_hash = BytesN::from_array(&env, &bytes);
        prop_assert!(attest_with_consent(&env, &client, &attester, &record_hash).is_ok());
        prop_assert!(client.get_attestation(&record_hash).is_some());
    }

    /// Attesting for an attester that was never allowlisted must fail
    /// cleanly with `AttesterNotAllowlisted` for any `record_hash`.
    #[test]
    fn attest_by_unknown_attester_never_panics(bytes in proptest::array::uniform32(any::<u8>())) {
        let env = Env::default();
        let (client, _attester_registry_client, _admin) = setup(&env);
        let attester = Address::generate(&env);
        let patient = Address::generate(&env);
        let record_hash = BytesN::from_array(&env, &bytes);

        let result = client.try_attest(&attester, &patient, &record_hash);
        prop_assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    }

    /// Re-attesting the same `record_hash` with arbitrary byte content, by
    /// several attesters, must never panic, and every attester's attestation
    /// remains in the retained history.
    #[test]
    fn repeated_attest_on_same_hash_never_panics(bytes in proptest::array::uniform32(any::<u8>()), attempts in 1usize..8) {
        let env = Env::default();
        let (client, attester_registry_client, admin) = setup(&env);

        let record_hash = BytesN::from_array(&env, &bytes);
        let mut attesters = std::vec::Vec::new();
        for _ in 0..attempts {
            let attester = Address::generate(&env);
            attester_registry_client.add_attester(&admin, &attester);
            prop_assert!(attest_with_consent(&env, &client, &attester, &record_hash).is_ok());
            attesters.push(attester);
        }

        let history = client.get_attestation_history(&record_hash);
        prop_assert_eq!(history.len() as usize, attempts);
        for expected in attesters {
            prop_assert!(history.iter().any(|attestation| attestation.attester == expected));
        }
    }

    /// `extend_attestation_ttl` must never shorten an existing expiry, must
    /// never push it past `MAX_TTL_HORIZON`, and must never mutate the
    /// attested data, for arbitrary requested horizons and callers.
    #[test]
    fn extend_ttl_is_monotonic_and_bounded(
        bytes in proptest::array::uniform32(any::<u8>()),
        requested in 0u64..(MAX_TTL_HORIZON * 4),
    ) {
        let env = Env::default();
        let (client, attester_registry_client, admin) = setup(&env);
        let attester = Address::generate(&env);
        attester_registry_client.add_attester(&admin, &attester);

        let record_hash = BytesN::from_array(&env, &bytes);
        let original = attest_with_consent(&env, &client, &attester, &record_hash)
            .expect("attestation should succeed");
        let original_expiry = original.expires_at;

        // Any caller may bump the TTL; use an unrelated address.
        let caller = Address::generate(&env);
        let now = env.ledger().timestamp();
        let extend_to = now.saturating_add(requested);
        let _ = client.try_extend_attestation_ttl(&caller, &record_hash, &extend_to);

        let after = client
            .get_attestation(&record_hash)
            .expect("attestation must still exist");
        // Data is never altered by a TTL bump.
        prop_assert_eq!(after.attester, original.attester);
        prop_assert_eq!(after.patient, original.patient);
        prop_assert_eq!(after.record_hash, original.record_hash);
        // Never shortened.
        prop_assert!(after.expires_at >= original_expiry);
        // Never beyond the capped horizon.
        prop_assert!(after.expires_at <= now.saturating_add(MAX_TTL_HORIZON));
    }

    /// Quorum aggregation must be monotonic under arbitrary interleavings of
    /// co-attestations and revocations: the distinct-attester count never
    /// exceeds the number of live attestations, never counts the same
    /// attester twice, and never increases when an attestation is revoked.
    #[test]
    fn quorum_count_is_monotonic_under_revocation(
        bytes in proptest::array::uniform32(any::<u8>()),
        ops in proptest::collection::vec(any::<bool>(), 1..12),
    ) {
        let env = Env::default();
        let (client, attester_registry_client, admin) = setup(&env);
        let record_hash = BytesN::from_array(&env, &bytes);

        let mut attesters = std::vec::Vec::new();
        for _ in 0..ops.len() {
            let attester = Address::generate(&env);
            attester_registry_client.add_attester(&admin, &attester);
            attesters.push(attester);
        }

        let mut live: std::vec::Vec<Address> = std::vec::Vec::new();
        let mut previous_count = 0u32;
        for (index, co_attest) in ops.iter().enumerate() {
            let attester = attesters[index].clone();
            if *co_attest {
                prop_assert!(attest_with_consent(&env, &client, &attester, &record_hash).is_ok());
                live.push(attester);
            } else if let Some(position) = live.iter().position(|a| *a == attester) {
                let _ = client.try_revoke(&attester, &record_hash);
                live.remove(position);
            }

            let view = client.get_verification(&record_hash);
            let count = view.distinct_attesters;
            // Never more distinct attesters than live attestations.
            prop_assert!(count <= live.len() as u32);
            // Revocation never increases the count.
            prop_assert!(count <= previous_count || *co_attest);
            previous_count = count;
        }
    }

    /// Disputes must be replay-safe and idempotent: malformed signatures are
    /// rejected without panicking, and repeated disputes by the same signer
    /// never inflate the dispute count or change the verification state.
    #[test]
    fn dispute_is_replay_safe_and_idempotent(
        bytes in proptest::array::uniform32(any::<u8>()),
        signature in proptest::array::uniform64(any::<u8>()),
        repeats in 1usize..6,
    ) {
        let env = Env::default();
        let (client, attester_registry_client, admin) = setup(&env);
        let attester = Address::generate(&env);
        attester_registry_client.add_attester(&admin, &attester);

        let record_hash = BytesN::from_array(&env, &bytes);
        prop_assert!(attest_with_consent(&env, &client, &attester, &record_hash).is_ok());

        let patient = Address::generate(&env);
        let malformed = BytesN::from_array(&env, &signature);
        // A malformed signature must fail cleanly, never panic.
        let _ = client.try_dispute_attestation(&patient, &record_hash, &malformed);
        prop_assert!(!client.get_verification(&record_hash).disputed);

        // A valid dispute is idempotent under repetition.
        let valid = client.sign_dispute(&patient, &record_hash);
        for _ in 0..repeats {
            let _ = client.try_dispute_attestation(&patient, &record_hash, &valid);
        }
        let view = client.get_verification(&record_hash);
        prop_assert!(view.disputed);
        prop_assert_eq!(view.dispute_count, 1);

        // Clearing requires attester or admin action.
        let _ = client.try_clear_dispute(&patient, &record_hash);
        prop_assert!(client.get_verification(&record_hash).disputed);
        let _ = client.try_clear_dispute(&admin, &record_hash);
        prop_assert!(!client.get_verification(&record_hash).disputed);
    }

    /// A status-list root must never be activated for an epoch unless every
    /// segment it references has been published for that same epoch. For
    /// arbitrary (epoch, segment) publish orders, activation either succeeds
    /// and the epoch is readable, or fails cleanly without panicking.
    #[test]
    fn status_list_root_requires_epoch_consistent_segments(
        epoch in any::<u64>(),
        segment_count in 1u32..8,
        publish_mask in proptest::collection::vec(any::<bool>(), 1..8),
    ) {
        let env = Env::default();
        let (client, _attester_registry_client, _admin) = setup(&env);

        let mut published = 0u32;
        for index in 0..segment_count {
            let should_publish = publish_mask
                .get(index as usize)
                .copied()
                .unwrap_or(false);
            if should_publish {
                let bits = BytesN::from_array(&env, &[0u8; 32]);
                let segment_hash = BytesN::from_array(&env, &[index as u8; 32]);
                let _ = client.try_publish_status_list_segment(
                    &epoch,
                    &index,
                    &bits,
                    &segment_hash,
                );
                published += 1;
            }
        }

        let root = BytesN::from_array(&env, &[0xABu8; 32]);
        let result = client.try_set_status_list_root(&epoch, &root);
        if published == segment_count {
            prop_assert!(result.is_ok());
            prop_assert_eq!(client.get_status_list_root(&epoch), Some(root));
        } else {
            // Incomplete segment set: activation must not silently succeed.
            prop_assert!(result.is_err() || client.get_status_list_root(&epoch).is_none());
        }
    }

    /// Revocation is monotonic across epochs: once a bit is set for a
    /// credential, publishing a later epoch that clears it must be rejected
    /// unless the explicit `unrevoke` path is used.
    #[test]
    fn revocation_is_monotonic_across_epochs(
        first_epoch in 0u64..1_000,
        epoch_gap in 1u64..100,
        bit_index in 0u32..8,
    ) {
        let env = Env::default();
        let (client, _attester_registry_client, _admin) = setup(&env);

        let later_epoch = first_epoch.saturating_add(epoch_gap);
        let mut revoked_bits = [0u8; 32];
        revoked_bits[(bit_index / 8) as usize] = 1u8 << (bit_index % 8);
        let revoked = BytesN::from_array(&env, &revoked_bits);
        let cleared = BytesN::from_array(&env, &[0u8; 32]);
        let segment_hash = BytesN::from_array(&env, &[0x11u8; 32]);

        // Publish the revocation in the first epoch.
        let _ = client.try_publish_status_list_segment(
            &first_epoch,
            &0u32,
            &revoked,
            &segment_hash,
        );

        // Attempting to clear the bit in a later epoch must not succeed
        // without the explicit unrevoke path.
        let clear_result = client.try_publish_status_list_segment(
            &later_epoch,
            &0u32,
            &cleared,
            &segment_hash,
        );
        prop_assert!(clear_result.is_err());

        // The explicit unrevoke path is the only way to clear it.
        let unrevoke_result = client.try_unrevoke_status_list_bit(&later_epoch, &bit_index);
        prop_assert!(unrevoke_result.is_ok());
    }

    /// Verifiers must always be able to read an epoch-consistent
    /// `(root, segments)` pair: whatever root is returned for an epoch, the
    /// segments published for that epoch are the ones referenced by it.
    #[test]
    fn verifier_reads_epoch_consistent_pair(
        epoch in any::<u64>(),
        segment_count in 1u32..6,
    ) {
        let env = Env::default();
        let (client, _attester_registry_client, _admin) = setup(&env);

        let mut segment_hashes = Vec::new(&env);
        for index in 0..segment_count {
            let bits = BytesN::from_array(&env, &[0u8; 32]);
            let segment_hash = BytesN::from_array(&env, &[index as u8; 32]);
            let _ = client.try_publish_status_list_segment(
                &epoch,
                &index,
                &bits,
                &segment_hash,
            );
            segment_hashes.push_back(segment_hash);
        }

        let root = BytesN::from_array(&env, &[0xCDu8; 32]);
        if client.try_set_status_list_root(&epoch, &root).is_ok() {
            prop_assert_eq!(client.get_status_list_root(&epoch), Some(root));
            for index in 0..segment_count {
                let stored = client.get_status_list_segment(&epoch, &index);
                prop_assert_eq!(stored, Some(segment_hashes.get(index).unwrap()));
            }
        }
    }
}
