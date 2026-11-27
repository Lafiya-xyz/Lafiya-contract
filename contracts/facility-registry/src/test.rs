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
// Cross-registry: stale affiliations after attester removal / suspension.
// ---------------------------------------------------------------------------

fn setup_with_attester_registry() -> (
    Env,
    FacilityRegistryClient<'static>,
    AttesterRegistryClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);
    let admin = Address::generate(&env);
    let facility_id = env.register(FacilityRegistry, (admin.clone(),));
    let attester_id = env.register(AttesterRegistry, (admin,));
    (
        env.clone(),
        FacilityRegistryClient::new(&env, &facility_id),
        AttesterRegistryClient::new(&env, &attester_id),
    )
}

fn register_attester(
    env: &Env,
    attesters: &AttesterRegistryClient<'static>,
) -> Address {
    let attester = Address::generate(env);
    attesters.register_attester(&attester, &license(env));
    attester
}

#[test]
fn suspended_attester_affiliation_is_not_effective() {
    let (env, facilities, attesters) = setup_with_attester_registry();
    let attester = register_attester(&env, &attesters);
    let id = facilities.enroll_facility(&license(&env), &None);
    facilities.affiliate_attester(&id, &attester);

    assert!(facilities.is_affiliated(&id, &attester));
    let effective = facilities.get_effective_affiliation(&id, &attester);
    assert!(effective.affiliated);
    assert!(effective.attester_active);
    assert!(effective.facility_active);

    attesters.suspend_attester(&attester);
    assert!(!facilities.is_affiliated(&id, &attester));
    let effective = facilities.get_effective_affiliation(&id, &attester);
    assert!(!effective.affiliated);
    assert!(!effective.attester_active);
    assert!(effective.facility_active);
    // History is preserved even though the affiliation is not effective.
    assert!(facilities.was_affiliated_at(&id, &attester, &150));
}

#[test]
fn removed_attester_affiliation_is_not_effective() {
    let (env, facilities, attesters) = setup_with_attester_registry();
    let attester = register_attester(&env, &attesters);
    let id = facilities.enroll_facility(&license(&env), &None);
    facilities.affiliate_attester(&id, &attester);

    attesters.remove_attester(&attester);
    assert!(!facilities.is_affiliated(&id, &attester));
    let effective = facilities.get_effective_affiliation(&id, &attester);
    assert!(!effective.affiliated);
    assert!(!effective.attester_active);
    assert!(effective.facility_active);
}

#[test]
fn reinstated_attester_affiliation_becomes_effective_again() {
    let (env, facilities, attesters) = setup_with_attester_registry();
    let attester = register_attester(&env, &attesters);
    let id = facilities.enroll_facility(&license(&env), &None);
    facilities.affiliate_attester(&id, &attester);

    attesters.suspend_attester(&attester);
    assert!(!facilities.is_affiliated(&id, &attester));

    attesters.reinstate_attester(&attester);
    assert!(facilities.is_affiliated(&id, &attester));
    let effective = facilities.get_effective_affiliation(&id, &attester);
    assert!(effective.affiliated);
    assert!(effective.attester_active);
    assert!(effective.facility_active);
}

#[test]
fn rotated_attester_key_does_not_inherit_affiliation() {
    let (env, facilities, attesters) = setup_with_attester_registry();
    let attester = register_attester(&env, &attesters);
    let id = facilities.enroll_facility(&license(&env), &None);
    facilities.affiliate_attester(&id, &attester);

    let new_key = Address::generate(&env);
    attesters.rotate_attester_key(&attester, &new_key);

    // The old key is no longer an active attester, so the affiliation is stale.
    assert!(!facilities.is_affiliated(&id, &attester));
    assert!(!facilities
        .get_effective_affiliation(&id, &attester)
        .attester_active);
    // The new key has no affiliation until explicitly granted.
    assert!(!facilities.is_affiliated(&id, &new_key));
    assert!(!facilities
        .get_effective_affiliation(&id, &new_key)
        .affiliated);
}

#[test]
fn attest_with_scope_rejected_after_attester_removal() {
    let (env, facilities, attesters) = setup_with_attester_registry();
    let attester = register_attester(&env, &attesters);
    let id = facilities.enroll_facility(&license(&env), &None);
    facilities.affiliate_attester(&id, &attester);

    // Sanity: the affiliation is effective before removal.
    assert!(facilities.is_affiliated(&id, &attester));

    attesters.remove_attester(&attester);
    assert!(facilities
        .try_attest_with_scope(&id, &attester)
        .is_err());
}

// ---------------------------------------------------------------------------
// Governance parity with attester-registry.
// ---------------------------------------------------------------------------

#[test]
fn pause_and_unpause_gate_mutations() {
    let (env, client) = setup();
    let admin = client.get_admin();
    let id = client.enroll_facility(&license(&env), &None);

    client.pause(&admin);
    assert!(client.is_paused());
    assert!(client
        .try_enroll_facility(&license(&env), &None)
        .is_err());
    assert!(client
        .try_affiliate_attester(&id, &Address::generate(&env))
        .is_err());

    client.unpause(&admin);
    assert!(!client.is_paused());
    assert_eq!(client.enroll_facility(&license(&env), &None), 2);
}

#[test]
fn pause_requires_admin() {
    let (env, client) = setup();
    let stranger = Address::generate(&env);
    assert!(client.try_pause(&stranger).is_err());
    assert!(!client.is_paused());
}

#[test]
fn two_step_admin_transfer() {
    let (env, client) = setup();
    let old_admin = client.get_admin();
    let new_admin = Address::generate(&env);

    client.propose_admin(&old_admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));
    // The old admin still governs until the proposal is accepted.
    assert_eq!(client.get_admin(), old_admin);
    assert!(client.try_pause(&new_admin).is_err());

    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);
    assert!(client.get_pending_admin().is_none());
    assert!(client.try_pause(&old_admin).is_err());
    client.pause(&new_admin);
    assert!(client.is_paused());
}

#[test]
fn admin_proposal_expires() {
    let (env, client) = setup();
    let old_admin = client.get_admin();
    let new_admin = Address::generate(&env);

    client.propose_admin(&old_admin, &new_admin);
    env.ledger()
        .set_timestamp(100 + ADMIN_PROPOSAL_TTL + 1);
    assert!(client.try_accept_admin(&new_admin).is_err());
    assert_eq!(client.get_admin(), old_admin);
}

#[test]
fn upgrade_and_migrate_bump_schema_version() {
    let (env, client) = setup();
    let admin = client.get_admin();
    let before = client.get_schema_version();

    let new_wasm = BytesN::from_array(&env, &[9; 32]);
    client.upgrade(&admin, &new_wasm);
    assert_eq!(client.get_schema_version(), before + 1);

    client.migrate(&admin, &(before + 1));
    assert_eq!(client.get_schema_version(), before + 1);
}

#[test]
fn upgrade_requires_admin() {
    let (env, client) = setup();
    let stranger = Address::generate(&env);
    let new_wasm = BytesN::from_array(&env, &[9; 32]);
    assert!(client.try_upgrade(&stranger, &new_wasm).is_err());
    assert!(client.try_migrate(&stranger, &1).is_err());
}
