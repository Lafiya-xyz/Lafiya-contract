//! Integration tests across attestation-registry, attester-registry and
//! facility-registry (issues #588, #589, #592).
extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Env, Map};

type ArClient = attester_registry::AttesterRegistryClient<'static>;
type FrClient = facility_registry::FacilityRegistryClient<'static>;

struct World {
    env: Env,
    client: AttestationRegistryClient<'static>,
    attesters: ArClient,
    facilities: FrClient,
    admin: Address,
}

fn world() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let admin = Address::generate(&env);
    let ar_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let attesters = ArClient::new(&env, &ar_id);
    attesters.grant_role(&attester_registry::Role::Registrar, &admin);
    attesters.grant_role(&attester_registry::Role::Guardian, &admin);
    let fr_id = env.register(facility_registry::FacilityRegistry, (admin.clone(),));
    let facilities = FrClient::new(&env, &fr_id);
    let id = env.register(AttestationRegistry, (admin.clone(), ar_id));
    let client = AttestationRegistryClient::new(&env, &id);
    client.set_facility_registry(&fr_id);
    World {
        env,
        client,
        attesters,
        facilities,
        admin,
    }
}

fn hash(env: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(env, &[n; 32])
}

fn consent(w: &World, attester: &Address, record: &BytesN<32>) -> Address {
    let patient = Address::generate(&w.env);
    let expires_at = w.env.ledger().timestamp() + 10_000;
    w.client
        .consent_attestation(&patient, attester, record, &expires_at);
    patient
}

fn sym(env: &Env, name: &str) -> Symbol {
    Symbol::new(env, name)
}

fn apply_matrix(w: &World) {
    let mut matrix: Map<Symbol, attester_registry::Tier> = Map::new(&w.env);
    matrix.set(
        sym(&w.env, "medication"),
        attester_registry::Tier::Physician,
    );
    w.attesters.propose_tier_matrix(&matrix);
    w.env.ledger().set_timestamp(
        w.env.ledger().timestamp() + attester_registry::TIER_MATRIX_TIMELOCK_SECONDS,
    );
    w.attesters.apply_tier_matrix();
}

// ── Tiers ──

#[test]
fn class_attestation_requires_sufficient_tier() {
    let w = world();
    apply_matrix(&w);
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let record = hash(&w.env, 1);
    let patient = consent(&w, &attester, &record);
    let medication = Some(sym(&w.env, "medication"));

    assert_eq!(
        w.client
            .try_attest_with_scope(&attester, &patient, &record, &medication, &None),
        Err(Ok(Error::AttesterTierInsufficient))
    );

    w.attesters
        .set_attester_tier(&w.admin, &attester, &attester_registry::Tier::Physician);
    w.client
        .attest_with_scope(&attester, &patient, &record, &medication, &None);
    let scope = w.client.get_attestation_scope(&record).unwrap();
    assert_eq!(scope.class, medication);
    assert_eq!(scope.facility_id, None);
}

#[test]
fn unclassified_and_unlisted_class_attestations_work_for_default_tier() {
    let w = world();
    apply_matrix(&w);
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let record = hash(&w.env, 2);
    let patient = consent(&w, &attester, &record);
    // Existing entry point is unaffected by the matrix.
    w.client.attest(&attester, &patient, &record);
    assert!(w.client.get_attestation_scope(&record).is_none());

    let record2 = hash(&w.env, 3);
    let patient2 = consent(&w, &attester, &record2);
    w.client.attest_with_scope(
        &attester,
        &patient2,
        &record2,
        &Some(sym(&w.env, "identity")),
        &None,
    );
}

// ── Facilities ──

#[test]
fn facility_attestation_requires_current_affiliation() {
    let w = world();
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let facility = w.facilities.enroll_facility(&hash(&w.env, 9), &None);
    let record = hash(&w.env, 4);
    let patient = consent(&w, &attester, &record);

    assert_eq!(
        w.client
            .try_attest_with_scope(&attester, &patient, &record, &None, &Some(facility)),
        Err(Ok(Error::FacilityNotAffiliated))
    );

    w.facilities.affiliate_attester(&facility, &attester);
    w.client
        .attest_with_scope(&attester, &patient, &record, &None, &Some(facility));
    assert_eq!(
        w.client.get_attestation_scope(&record).unwrap().facility_id,
        Some(facility)
    );
}

#[test]
fn disaffiliation_does_not_rewrite_existing_attestations() {
    let w = world();
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let facility = w.facilities.enroll_facility(&hash(&w.env, 9), &None);
    w.facilities.affiliate_attester(&facility, &attester);
    let record = hash(&w.env, 5);
    let patient = consent(&w, &attester, &record);
    let attestation =
        w.client
            .attest_with_scope(&attester, &patient, &record, &None, &Some(facility));

    w.env.ledger().set_timestamp(attestation.timestamp + 100);
    w.facilities.disaffiliate_attester(&facility, &attester);

    // The attestation and its stored scope stay; the verifier can still
    // prove affiliation at the time it was made.
    assert!(w.client.is_verified(&record));
    assert_eq!(
        w.client.get_attestation_scope(&record).unwrap().facility_id,
        Some(facility)
    );
    assert!(w
        .facilities
        .was_affiliated_at(&facility, &attester, &attestation.timestamp));
    assert!(!w.facilities.is_affiliated(&facility, &attester));

    // New facility-scoped attestations are refused.
    let record2 = hash(&w.env, 6);
    let patient2 = consent(&w, &attester, &record2);
    assert_eq!(
        w.client
            .try_attest_with_scope(&attester, &patient2, &record2, &None, &Some(facility)),
        Err(Ok(Error::FacilityNotAffiliated))
    );
}

#[test]
fn deactivated_facility_refuses_new_scoped_attestations() {
    let w = world();
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let facility = w.facilities.enroll_facility(&hash(&w.env, 9), &None);
    w.facilities.affiliate_attester(&facility, &attester);
    w.facilities.deactivate_facility(&facility);
    let record = hash(&w.env, 7);
    let patient = consent(&w, &attester, &record);
    assert_eq!(
        w.client
            .try_attest_with_scope(&attester, &patient, &record, &None, &Some(facility)),
        Err(Ok(Error::FacilityNotAffiliated))
    );
}

#[test]
fn facility_scope_needs_a_configured_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let ar_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let attesters = ArClient::new(&env, &ar_id);
    attesters.grant_role(&attester_registry::Role::Registrar, &admin);
    let id = env.register(AttestationRegistry, (admin.clone(), ar_id));
    let client = AttestationRegistryClient::new(&env, &id);
    let attester = Address::generate(&env);
    attesters.add_attester(&admin, &attester);
    let record = hash(&env, 1);
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record, &expires_at);
    assert_eq!(
        client.try_attest_with_scope(&attester, &patient, &record, &None, &Some(1)),
        Err(Ok(Error::FacilityRegistryNotSet))
    );
}

#[test]
fn plain_reattestation_clears_the_previous_scope() {
    let w = world();
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);
    let facility = w.facilities.enroll_facility(&hash(&w.env, 9), &None);
    w.facilities.affiliate_attester(&facility, &attester);
    let record = hash(&w.env, 8);
    let patient = consent(&w, &attester, &record);
    w.client
        .attest_with_scope(&attester, &patient, &record, &None, &Some(facility));
    let patient2 = consent(&w, &attester, &record);
    w.client.attest(&attester, &patient2, &record);
    assert!(w.client.get_attestation_scope(&record).is_none());
}

// ── Probation rate cap ──

#[test]
fn probation_rate_cap_limits_attestations_until_promotion() {
    let w = world();
    w.attesters
        .set_probation_config(&(30 * 24 * 60 * 60), &2, &100, &true);
    let attester = Address::generate(&w.env);
    w.attesters.add_attester(&w.admin, &attester);

    for n in 0..2u8 {
        let record = hash(&w.env, 20 + n);
        let patient = consent(&w, &attester, &record);
        w.client.attest(&attester, &patient, &record);
    }
    let record = hash(&w.env, 30);
    let patient = consent(&w, &attester, &record);
    assert_eq!(
        w.client.try_attest(&attester, &patient, &record),
        Err(Ok(Error::ProbationRateLimited))
    );

    // The cap window rolls over.
    w.env.ledger().with_mut(|l| l.sequence_number += 100);
    w.client.attest(&attester, &patient, &record);

    // Early promotion lifts the cap.
    w.attesters.promote_attester(&w.admin, &attester);
    for n in 0..3u8 {
        let record = hash(&w.env, 40 + n);
        let patient = consent(&w, &attester, &record);
        w.client.attest(&attester, &patient, &record);
    }
}
