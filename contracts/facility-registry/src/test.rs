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
fn affiliation_period_history_is_paged_and_bounded() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    let cycles: u32 = 1_000;
    for i in 0..cycles {
        env.ledger().set_timestamp(1_000 + (i as u64) * 10);
        client.affiliate_attester(&id, &attester);
        env.ledger().set_timestamp(1_005 + (i as u64) * 10);
        client.disaffiliate_attester(&id, &attester);
    }
    // Every page entry stays within the fixed page size.
    let pages = client.get_affiliation_page_count(&id, &attester);
    assert!(pages >= 1);
    for p in 0..pages {
        let page = client.get_affiliation_page(&id, &attester, &p);
        assert!(page.len() <= AFFILIATION_PAGE_SIZE);
    }
    // The header only holds the latest period.
    assert_eq!(client.get_affiliation_periods(&id, &attester, &0, &1).len(), 1);
}

#[test]
fn was_affiliated_at_is_correct_across_page_boundaries() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    let cycles: u32 = AFFILIATION_PAGE_SIZE * 2 + 3;
    for i in 0..cycles {
        env.ledger().set_timestamp(1_000 + (i as u64) * 10);
        client.affiliate_attester(&id, &attester);
        env.ledger().set_timestamp(1_005 + (i as u64) * 10);
        client.disaffiliate_attester(&id, &attester);
    }
    for i in 0..cycles {
        let start = 1_000 + (i as u64) * 10;
        let end = 1_005 + (i as u64) * 10;
        assert!(client.was_affiliated_at(&id, &attester, &start));
        assert!(client.was_affiliated_at(&id, &attester, &(end - 1)));
        assert!(!client.was_affiliated_at(&id, &attester, &end));
        assert!(!client.was_affiliated_at(&id, &attester, &(start - 1)));
    }
}

#[test]
fn get_affiliation_periods_paginates_with_cursor_and_limit() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    for i in 0..5u64 {
        env.ledger().set_timestamp(1_000 + i * 10);
        client.affiliate_attester(&id, &attester);
        env.ledger().set_timestamp(1_005 + i * 10);
        client.disaffiliate_attester(&id, &attester);
    }
    let first = client.get_affiliation_periods(&id, &attester, &0, &2);
    assert_eq!(first.len(), 2);
    assert_eq!(first.get(0).unwrap().since, 1_000);
    assert_eq!(first.get(1).unwrap().since, 1_010);
    let second = client.get_affiliation_periods(&id, &attester, &2, &2);
    assert_eq!(second.len(), 2);
    assert_eq!(second.get(0).unwrap().since, 1_020);
    let last = client.get_affiliation_periods(&id, &attester, &4, &2);
    assert_eq!(last.len(), 1);
    assert_eq!(last.get(0).unwrap().since, 1_040);
    assert_eq!(client.get_affiliation_periods(&id, &attester, &5, &2).len(), 0);
}

#[test]
fn deactivation_records_reason_and_timestamp() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    client.affiliate_attester(&id, &attester);
    env.ledger().set_timestamp(500);
    client.deactivate_facility(&id, &DeactivationReason::Closed);
    let info = client.get_facility(&id).unwrap();
    assert_eq!(info.deactivated_at, Some(500));
    assert_eq!(info.deactivation_reason, Some(DeactivationReason::Closed));
    // Non-fraud deactivation keeps past attestations valid.
    assert!(client.was_affiliated_at(&id, &attester, &400));
    assert_eq!(
        client.get_attestation_facility_status(&id, &400),
        FacilityStatus::Valid
    );
}

#[test]
fn fraud_deactivation_marks_past_attestations_untrusted() {
    let (env, client) = setup();
    let attester = Address::generate(&env);
    let id = client.enroll_facility(&license(&env), &None);
    client.affiliate_attester(&id, &attester);
    env.ledger().set_timestamp(500);
    client.deactivate_facility(&id, &DeactivationReason::Fraud);
    let info = client.get_facility(&id).unwrap();
    assert_eq!(info.deactivation_reason, Some(DeactivationReason::Fraud));
    assert_eq!(
        client.get_attestation_facility_status(&id, &400),
        FacilityStatus::FacilityUntrusted
    );
    assert_eq!(
        client.get_attestation_facility_status(&id, &500),
        FacilityStatus::FacilityUntrusted
    );
}

#[test]
fn portable_facility_id_derivation_and_lookup() {
    let (env, client) = setup();
    let jurisdiction = BytesN::from_array(&env, &[2; 32]);
    let authority = BytesN::from_array(&env, &[3; 32]);
    let code = BytesN::from_array(&env, &[4; 32]);
    let expected = derive_facility_id(&env, &jurisdiction, &authority, &code);
    let id = client.enroll_facility_portable(&jurisdiction, &authority, &code, &None);
    assert_eq!(id, expected);
    assert_eq!(client.get_facility_by_portable_id(&expected).unwrap(), 1);
    assert_eq!(client.get_facility_by_external_code_hash(&code), Some(1));
    // u64 alias remains for compatibility.
    assert_eq!(client.get_facility(&1).unwrap().portable_id, expected);
    assert!(client.get_facility_by_portable_id(&BytesN::from_array(&env, &[9; 32])).is_none());
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
