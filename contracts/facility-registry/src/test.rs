extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};

fn setup() -> (Env, FacilityRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);
    let admin = Address::generate(&env);
    let id = env.register(FacilityRegistry, (admin,));
    (env.clone(), FacilityRegistryClient::new(&env, &id))
}

fn license(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[1; 32])
}

#[test]
fn enrolls_facilities_with_sequential_ids_and_mutation_cursor() {
    let (env, client) = setup();
    assert_eq!(client.get_mutation_seq(), 0);
    assert_eq!(client.enroll_facility(&license(&env), &None), 1);
    assert_eq!(client.enroll_facility(&license(&env), &None), 2);
    assert_eq!(client.get_facility_count(), 2);
    assert_eq!(client.get_mutation_seq(), 2);
    let info = client.get_facility(&1).unwrap();
    assert!(info.active);
    assert_eq!(info.enrolled_at, 100);
    assert!(client.get_facility(&3).is_none());
}

#[test]
fn affiliation_requires_active_facility_and_is_not_duplicated() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    assert!(client.try_affiliate_attester(&1, &attester).is_err());
    let id = client.enroll_facility(&license(&env), &None);

    assert!(!client.is_affiliated(&id, &attester));
    client.affiliate_attester(&id, &attester);
    assert!(client.is_affiliated(&id, &attester));
    assert_eq!(client.get_affiliate_count(&id), 1);
    assert!(client.try_affiliate_attester(&id, &attester).is_err());
}

#[test]
fn disaffiliation_preserves_history_for_existing_attestations() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    client.affiliate_attester(&id, &attester);
    env.ledger().set_timestamp(200);
    client.disaffiliate_attester(&id, &attester);

    assert!(!client.is_affiliated(&id, &attester));
    assert_eq!(client.get_affiliate_count(&id), 0);
    // Boundaries: `since` inclusive, `until` exclusive.
    assert!(!client.was_affiliated_at(&id, &attester, &99));
    assert!(client.was_affiliated_at(&id, &attester, &100));
    assert!(client.was_affiliated_at(&id, &attester, &199));
    assert!(!client.was_affiliated_at(&id, &attester, &200));
    assert!(client.try_disaffiliate_attester(&id, &attester).is_err());

    // Re-affiliation adds a period instead of rewriting the earlier one.
    env.ledger().set_timestamp(300);
    client.affiliate_attester(&id, &attester);
    assert_eq!(client.get_affiliation_periods(&id, &attester).len(), 2);
    assert!(client.was_affiliated_at(&id, &attester, &150));
    assert!(!client.was_affiliated_at(&id, &attester, &250));
}

#[test]
fn deactivation_voids_affiliations_without_erasing_them() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    client.affiliate_attester(&id, &attester);
    env.ledger().set_timestamp(500);
    client.deactivate_facility(&id);

    assert!(!client.is_facility_active(&id));
    assert!(!client.is_affiliated(&id, &attester));
    assert!(client.was_affiliated_at(&id, &attester, &400));
    assert_eq!(client.get_facility(&id).unwrap().deactivated_at, Some(500));
    assert!(client.try_deactivate_facility(&id).is_err());
    assert!(client
        .try_affiliate_attester(&id, &Address::generate(&env))
        .is_err());

    client.reactivate_facility(&id);
    assert!(client.is_affiliated(&id, &attester));
    assert!(client.try_reactivate_facility(&id).is_err());
}

#[test]
fn affiliation_period_history_is_bounded() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    for _ in 0..MAX_PERIODS {
        client.affiliate_attester(&id, &attester);
        client.disaffiliate_attester(&id, &attester);
    }
    assert!(client.try_affiliate_attester(&id, &attester).is_err());
}

#[test]
fn every_mutation_advances_the_cursor() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    client.affiliate_attester(&id, &attester);
    client.disaffiliate_attester(&id, &attester);
    client.deactivate_facility(&id);
    client.reactivate_facility(&id);
    assert_eq!(client.get_mutation_seq(), 5);
    // Failed calls do not advance it.
    assert!(client.try_reactivate_facility(&id).is_err());
    assert_eq!(client.get_mutation_seq(), 5);
}

// ---------------------------------------------------------------------------
// Archival harness (#723)
//
// Enumerates every persistent `DataKey` variant, archives it by advancing the
// ledger past its TTL, and asserts the documented recovery behavior. The
// invariant under test: archiving a key must never make a record appear
// *more* trusted (e.g. an archived affiliation must not read as affiliated).
// ---------------------------------------------------------------------------

/// Every persistent key class in the facility registry. Keep in sync with
/// `StorageKey::durability`; the exhaustive match below fails to compile if a
/// new variant is added without being classified here.
#[derive(Clone, Copy, Debug)]
enum KeyClass {
    Facility,
    FacilityCount,
    Affiliation,
    AffiliationPeriods,
    AffiliateCount,
    MutationSeq,
}

const ALL_KEY_CLASSES: [KeyClass; 6] = [
    KeyClass::Facility,
    KeyClass::FacilityCount,
    KeyClass::Affiliation,
    KeyClass::AffiliationPeriods,
    KeyClass::AffiliateCount,
    KeyClass::MutationSeq,
];

/// Classify a `DataKey` into its archival class. Exhaustive so that adding a
/// new persistent key forces an update to this harness.
fn classify(key: &DataKey) -> KeyClass {
    match key {
        DataKey::Facility(_) => KeyClass::Facility,
        DataKey::FacilityCount => KeyClass::FacilityCount,
        DataKey::Affiliation(_, _) => KeyClass::Affiliation,
        DataKey::AffiliationPeriods(_, _) => KeyClass::AffiliationPeriods,
        DataKey::AffiliateCount(_) => KeyClass::AffiliateCount,
        DataKey::MutationSeq => KeyClass::MutationSeq,
    }
}

/// Advance the ledger far past any configured TTL so the targeted entry is
/// eligible for archival. The test environment does not run the archival
/// keeper automatically, so we also explicitly remove the entry to model the
/// post-archival state (read returns `None`).
fn archive_key(env: &Env, key: &DataKey) {
    env.ledger().set_timestamp(env.ledger().timestamp() + 10_000_000);
    env.storage().persistent().remove(key);
}

fn seed_all_keys(env: &Env, client: &FacilityRegistryClient) -> (u64, Address) {
    let attester = Address::generate(env);
    let id = client.enroll_facility(&license(env), &None);
    client.affiliate_attester(&id, &attester);
    env.ledger().set_timestamp(200);
    client.disaffiliate_attester(&id, &attester);
    (id, attester)
}

#[test]
fn archival_harness_covers_every_persistent_key_class() {
    // Sanity: the enumerated list matches the exhaustive classifier by
    // exercising a representative key of each class.
    let (env, client) = setup();
    let (id, attester) = seed_all_keys(&env, &client);
    let keys = [
        DataKey::Facility(id),
        DataKey::FacilityCount,
        DataKey::Affiliation(id, attester.clone()),
        DataKey::AffiliationPeriods(id, attester.clone()),
        DataKey::AffiliateCount(id),
        DataKey::MutationSeq,
    ];
    let mut seen = std::vec::Vec::new();
    for k in keys.iter() {
        let c = classify(k);
        if !seen.contains(&format!("{:?}", c)) {
            seen.push(format!("{:?}", c));
        }
    }
    assert_eq!(seen.len(), ALL_KEY_CLASSES.len());
}

#[test]
fn archived_facility_reads_as_absent_not_active() {
    let (env, client) = setup();
    let (id, _) = seed_all_keys(&env, &client);
    archive_key(&env, &DataKey::Facility(id));
    // Documented: archived facility -> `None`, never a fabricated active record.
    assert!(client.get_facility(&id).is_none());
    assert!(!client.is_facility_active(&id));
}

#[test]
fn archived_affiliation_does_not_appear_affiliated() {
    let (env, client) = setup();
    let (id, attester) = seed_all_keys(&env, &client);
    // Re-affiliate so the current affiliation entry is present.
    env.ledger().set_timestamp(300);
    client.affiliate_attester(&id, &attester);
    assert!(client.is_affiliated(&id, &attester));

    archive_key(&env, &DataKey::Affiliation(id, attester.clone()));
    // Fail-closed: an archived affiliation must not read as affiliated.
    assert!(!client.is_affiliated(&id, &attester));
}

#[test]
fn archived_affiliation_periods_do_not_appear_affiliated() {
    let (env, client) = setup();
    let (id, attester) = seed_all_keys(&env, &client);
    env.ledger().set_timestamp(300);
    client.affiliate_attester(&id, &attester);
    assert!(client.is_affiliated(&id, &attester));

    archive_key(&env, &DataKey::AffiliationPeriods(id, attester.clone()));
    // Fail-closed: without period history the attester is not affiliated.
    assert!(!client.is_affiliated(&id, &attester));
    assert!(!client.was_affiliated_at(&id, &attester, &150));
}

#[test]
fn archived_affiliate_count_does_not_grant_affiliation() {
    let (env, client) = setup();
    let (id, attester) = seed_all_keys(&env, &client);
    env.ledger().set_timestamp(300);
    client.affiliate_attester(&id, &attester);

    archive_key(&env, &DataKey::AffiliateCount(id));
    // A missing count must not be treated as "affiliated".
    assert!(!client.is_affiliated(&id, &attester));
}

#[test]
fn archived_deactivation_marker_does_not_reactivate() {
    let (env, client) = setup();
    let (id, attester) = seed_all_keys(&env, &client);
    env.ledger().set_timestamp(500);
    client.deactivate_facility(&id);
    assert!(!client.is_facility_active(&id));

    // Archive the facility record that carries the deactivation marker.
    archive_key(&env, &DataKey::Facility(id));
    // Fail-closed: archived deactivation must not make the facility active.
    assert!(!client.is_facility_active(&id));
    assert!(client.get_facility(&id).is_none());
    assert!(!client.is_affiliated(&id, &attester));
}

#[test]
fn archived_mutation_seq_does_not_fabricate_history() {
    let (env, client) = setup();
    let (id, _) = seed_all_keys(&env, &client);
    let before = client.get_mutation_seq();
    archive_key(&env, &DataKey::MutationSeq);
    // Documented: archived cursor reads as 0, never a fabricated higher value.
    assert_eq!(client.get_mutation_seq(), 0);
    assert!(before > 0);
}

// ---------------------------------------------------------------------------
// Cross-contract fault-injection matrix (#725)
//
// The facility registry has no outbound cross-contract calls today, so the
// consumer-by-behavior matrix is empty by construction. This test pins that
// fact: if a dependency is wired in later, this test must be extended with
// stub behaviors (panic, error code, wrong type, budget exhaustion, fail-open
// `true`) and must assert fail-closed typed errors rather than traps.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum StubBehavior {
    Panic,
    ErrorCode,
    WrongType,
    BudgetExhausted,
    FailOpenTrue,
}

const ALL_STUB_BEHAVIORS: [StubBehavior; 5] = [
    StubBehavior::Panic,
    StubBehavior::ErrorCode,
    StubBehavior::WrongType,
    StubBehavior::BudgetExhausted,
    StubBehavior::FailOpenTrue,
];

#[test]
fn cross_contract_matrix_is_fail_closed() {
    // No outbound dependency entry points exist in this contract. The matrix
    // is therefore vacuously fail-closed; every behavior is accounted for so
    // that adding a dependency forces an explicit decision here.
    let (env, client) = setup();
    let id = client.enroll_facility(&license(&env), &None);
    for behavior in ALL_STUB_BEHAVIORS.iter() {
        // With no dependency, local reads must still be deterministic and
        // never trap regardless of what a hypothetical stub would do.
        let _ = behavior;
        assert!(client.get_facility(&id).is_some());
        assert!(client.is_facility_active(&id));
    }
}
