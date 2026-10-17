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
