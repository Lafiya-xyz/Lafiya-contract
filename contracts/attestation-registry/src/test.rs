extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{BytesN, Env, Event, IntoVal};

fn setup() -> (
    Env,
    AttestationRegistryClient<'static>,
    attester_registry::AttesterRegistryClient<'static>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    attester_registry_client.initialize(&admin);
    attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);
    client.initialize(&admin, &attester_registry_id);
    client.grant_role(&Role::Guardian, &admin);
    client.grant_role(&Role::Revoker, &admin);

    (env, client, attester_registry_client, admin)
}

fn attest_with_consent(
    env: &Env,
    client: &AttestationRegistryClient<'_>,
    attester: &Address,
    record_hash: &BytesN<32>,
) -> Attestation {
    let patient = Address::generate(env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, attester, record_hash, &expires_at);
    client.attest(attester, &patient, record_hash)
}

#[test]
fn configuration_getters_return_initialized_addresses() {
    let (_env, client, attester_registry, admin) = setup();

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_attester_registry(), attester_registry.address);
    assert_eq!(client.get_schema_version(), SCHEMA_VERSION);
}

#[test]
fn migrate_updates_a_legacy_storage_version() {
    let (env, client, _attester_registry, _admin) = setup();
    env.as_contract(&client.address, || {
        env.storage().instance().set(&DataKey::SchemaVersion, &1u32);
    });

    assert_eq!(client.get_schema_version(), 1);
    client.migrate();
    assert_eq!(client.get_schema_version(), SCHEMA_VERSION);
}

#[test]
fn migrate_rejects_current_schema_version() {
    let (_env, client, _attester_registry, _admin) = setup();
    assert_eq!(client.try_migrate(), Err(Ok(Error::MigrationNotRequired)));
}

#[test]
fn configuration_getters_before_initialize_fail() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);

    assert_eq!(client.try_get_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(
        client.try_get_attester_registry(),
        Err(Ok(Error::NotInitialized))
    );
}

#[test]
fn get_admin_before_initialize_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);

    let result = client.try_get_admin();
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn get_attester_registry_before_initialize_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);

    let result = client.try_get_attester_registry();
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn attest_by_allowlisted_attester_succeeds() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester_with_info(
        &_admin,
        &attester,
        &None,
        &Some(Symbol::new(&env, "lagos")),
        &None,
        &None,
    );

    let record_hash = BytesN::from_array(&env, &[7u8; 32]);
    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(attestation.commitment_version, 0);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn attest_requires_matching_patient_consent() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[71u8; 32]);

    let result = client.try_attest(&attester, &patient, &record_hash);
    assert_eq!(result, Err(Ok(Error::PatientConsentRequired)));

    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    let other_patient = Address::generate(&env);
    let mismatched = client.try_attest(&attester, &other_patient, &record_hash);
    assert_eq!(mismatched, Err(Ok(Error::PatientConsentRequired)));

    let attestation = client.attest(&attester, &patient, &record_hash);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));

    let reused = client.try_attest(&attester, &patient, &record_hash);
    assert_eq!(reused, Err(Ok(Error::PatientConsentRequired)));
}

#[test]
fn patient_consent_requires_patient_authorization() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[72u8; 32]);

    env.mock_auths(&[]);
    let expires_at = env.ledger().timestamp() + 10_000;
    let result = client.try_consent_attestation(&patient, &attester, &record_hash, &expires_at);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_by_non_allowlisted_attester_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[1u8; 32]);

    let patient = Address::generate(&env);
    let result = client.try_attest(&attester, &patient, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn anchor_batch_stores_one_root_and_returns_metadata() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let root = BytesN::from_array(&env, &[21u8; 32]);

    let batch = client.anchor_batch(&attester, &root, &3);

    assert_eq!(batch.attester, attester);
    assert_eq!(batch.leaf_count, 3);
    assert_eq!(client.get_attestation_batch(&root), Some(batch));
}

#[test]
fn anchor_batch_rejects_empty_batch() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let root = BytesN::from_array(&env, &[22u8; 32]);

    assert_eq!(
        client.try_anchor_batch(&attester, &root, &0),
        Err(Ok(Error::EmptyBatch))
    );
    assert_eq!(client.get_attestation_batch(&root), None);
}

#[test]
fn anchor_batch_rejects_non_allowlisted_attester() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let root = BytesN::from_array(&env, &[23u8; 32]);

    assert_eq!(
        client.try_anchor_batch(&attester, &root, &2),
        Err(Ok(Error::AttesterNotAllowlisted))
    );
    assert_eq!(client.get_attestation_batch(&root), None);
}

#[test]
fn anchor_batch_rejects_reused_root() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let root = BytesN::from_array(&env, &[24u8; 32]);
    client.anchor_batch(&attester, &root, &2);

    assert_eq!(
        client.try_anchor_batch(&attester, &root, &2),
        Err(Ok(Error::BatchAlreadyAnchored))
    );
}

#[test]
fn attest_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[2u8; 32]);

    let patient = Address::generate(&env);
    let result = client.try_attest(&attester, &patient, &record_hash);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn attest_returns_registry_unavailable_when_registry_call_traps() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[12u8; 32]);
    let broken_registry = Address::generate(&env);
    client.set_attester_registry(&broken_registry);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttesterRegistryUnavailable)));
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn get_attestation_returns_none_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[9u8; 32]);
    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(client.get_attestation_status(&record_hash), None);
}

#[test]
fn get_attestations_returns_results_in_input_order_including_misses() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let first_hash = BytesN::from_array(&env, &[15u8; 32]);
    let missing_hash = BytesN::from_array(&env, &[16u8; 32]);
    let last_hash = BytesN::from_array(&env, &[17u8; 32]);
    let first = client.attest(&attester, &first_hash);
    let last = client.attest(&attester, &last_hash);

    let hashes = Vec::from_array(
        &env,
        [first_hash.clone(), missing_hash, last_hash.clone()],
    );
    let results = client.get_attestations(&hashes);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0), Some(Some(first)));
    assert_eq!(results.get(1), Some(None));
    assert_eq!(results.get(2), Some(Some(last)));
}

#[test]
fn is_verified_applies_age_and_current_attester_status() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[13u8; 32]);

    assert!(!client.is_verified(&record_hash));
    assert_eq!(client.get_max_attestation_age(), DEFAULT_MAX_ATTESTATION_AGE);

    attester_registry.add_attester(&attester);
    let attestation = client.attest(&attester, &record_hash);
    assert!(client.is_verified(&record_hash));

    attester_registry.suspend_attester(&attester);
    assert!(!client.is_verified(&record_hash));
    attester_registry.reinstate_attester(&attester);
    assert!(client.is_verified(&record_hash));

    client.set_max_attestation_age(&0);
    assert_eq!(client.get_max_attestation_age(), 0);
    env.ledger()
        .set_timestamp(attestation.timestamp.saturating_add(1));
    assert!(!client.is_verified(&record_hash));

    client.set_max_attestation_age(&DEFAULT_MAX_ATTESTATION_AGE);
    attester_registry.remove_attester(&attester);
    assert!(!client.is_verified(&record_hash));

    client.revoke_attestation(&record_hash);
    assert!(!client.is_verified(&record_hash));
}

#[test]
fn is_verified_reports_broken_registry_call() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[14u8; 32]);
    client.attest(&attester, &record_hash);
    client.set_attester_registry(&Address::generate(&env));

    assert_eq!(
        client.try_is_verified(&record_hash),
        Err(Ok(Error::AttesterRegistryUnavailable))
    );
}

#[test]
fn re_attest_overwrites_previous_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&_admin, &attester_a);
    attester_registry.add_attester(&_admin, &attester_b);

    let record_hash = BytesN::from_array(&env, &[3u8; 32]);
    let first = attest_with_consent(&env, &client, &attester_a, &record_hash);
    let second = attest_with_consent(&env, &client, &attester_b, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), Some(second.clone()));

    // The overwritten attestation must not be dropped: it should remain in
    // the history as the older entry, with the new one appended after it.
    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0), Some(first));
    assert_eq!(history.get(1), Some(second));
}

#[test]
fn get_attestation_history_returns_all_attestations() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    let attester_c = Address::generate(&env);
    attester_registry.add_attester(&_admin, &attester_a);
    attester_registry.add_attester(&_admin, &attester_b);
    attester_registry.add_attester(&_admin, &attester_c);

    let record_hash = BytesN::from_array(&env, &[6u8; 32]);
    let first = attest_with_consent(&env, &client, &attester_a, &record_hash);
    let second = attest_with_consent(&env, &client, &attester_b, &record_hash);
    let third = attest_with_consent(&env, &client, &attester_c, &record_hash);

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 3);
    assert_eq!(history.get(0), Some(first));
    assert_eq!(history.get(1), Some(second));
    assert_eq!(history.get(2), Some(third));
}

#[test]
fn attester_can_withdraw_only_their_own_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);

    let record_hash = BytesN::from_array(&env, &[42u8; 32]);
    let first = client.attest(&attester_a, &record_hash);
    let first_again = client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);

    client.withdraw_attestation(&attester_a, &record_hash);

    let expected_event = AttestationWithdrawn {
        record_hash: record_hash.clone(),
        attester: attester_a.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)]
    );
    assert_eq!(client.get_attestation(&record_hash), Some(second));
    assert_eq!(
        client.get_attestation_status(&record_hash),
        AttestationStatus::Verified
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester_a),
        AttesterAttestationStatus::Withdrawn
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester_b),
        AttesterAttestationStatus::Active
    );
    assert_eq!(client.get_attestation_history(&record_hash).len(), 3);
    // The attestation is retained as an immutable historical record.
    assert_eq!(
        client.get_attestation_history(&record_hash).get(0),
        Some(first)
    );
    assert_eq!(
        client.get_attestation_history(&record_hash).get(1),
        Some(first_again)
    );
}

#[test]
fn attester_cannot_withdraw_another_attesters_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let owner = Address::generate(&env);
    let other = Address::generate(&env);
    attester_registry.add_attester(&owner);
    attester_registry.add_attester(&other);

    let record_hash = BytesN::from_array(&env, &[43u8; 32]);
    let attestation = client.attest(&owner, &record_hash);

    let result = client.try_withdraw_attestation(&other, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttestationNotOwned)));
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn withdrawing_the_only_attestation_reports_withdrawn() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[45u8; 32]);
    client.attest(&attester, &record_hash);
    client.attest(&attester, &record_hash);
    client.withdraw_attestation(&attester, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(client.get_attestation_history(&record_hash).len(), 2);
    assert_eq!(
        client.get_attestation_status(&record_hash),
        AttestationStatus::Withdrawn
    );
}

#[test]
fn attester_withdrawal_requires_the_attesters_authorization() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[44u8; 32]);
    let attestation = client.attest(&attester, &record_hash);

    env.mock_auths(&[]);
    let result = client.try_withdraw_attestation(&attester, &record_hash);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn get_attestation_history_returns_empty_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[8u8; 32]);
    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 0);
}

#[test]
fn attestation_history_is_bounded() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[10u8; 32]);

    for _ in 0..15 {
        attest_with_consent(&env, &client, &attester, &record_hash);
    }

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 10);
}

#[test]
fn get_attestation_history_boundary_at_max_history() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[11u8; 32]);

    let mut attestations = std::vec![];
    for _ in 0..11 {
        attestations.push(attest_with_consent(&env, &client, &attester, &record_hash));
    }

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 10);

    for i in 0..10 {
        let idx = i as usize + 1;
        assert_eq!(
            history.get(i).unwrap().timestamp,
            attestations[(i + 1) as usize].timestamp
        );
    }
    assert_eq!(
        client.get_attestation(&record_hash),
        Some(attestations[10].clone())
    );
}

#[test]
fn patient_selected_expiry_marks_attestation_stale() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[15u8; 32]);
    attester_registry.add_attester(&attester);

    let expires_at = env.ledger().timestamp() + 10;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    let attestation = client.attest(&attester, &patient, &record_hash);

    let fresh = client.get_attestation_status(&record_hash).unwrap();
    assert_eq!(fresh.attestation, attestation);
    assert_eq!(fresh.expires_at, Some(expires_at));
    assert!(!fresh.is_expired);

    env.ledger()
        .with_mut(|ledger| ledger.timestamp = expires_at);
    let stale = client.get_attestation_status(&record_hash).unwrap();
    assert!(stale.is_expired);
    let history = client.get_attestation_history_status(&record_hash);
    assert_eq!(history.len(), 1);
    assert!(history.get(0).unwrap().is_expired);
}

#[test]
fn patient_cannot_grant_already_expired_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[16u8; 32]);
    attester_registry.add_attester(&attester);

    let expires_at = env.ledger().timestamp();
    assert_eq!(
        client.try_consent_attestation(&patient, &attester, &record_hash, &expires_at),
        Err(Ok(Error::InvalidAttestationExpiry))
    );
}

#[test]
fn attest_emits_event() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&_admin, &attester);
    let record_hash = BytesN::from_array(&env, &[4u8; 32]);

    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    let expected_event = AttestationRecorded {
        record_hash: record_hash.clone(),
        attester: attestation.attester.clone(),
        timestamp: attestation.timestamp,
        expires_at: env.ledger().timestamp() + 10_000,
    };
    assert_eq!(
        env.events().all(),
        std::vec![
            AdminTransferProposed {
                current_admin: admin.clone(),
                proposed_admin: new_admin.clone(),
                expires_at: 30 * 24 * 60 * 60,
            }
            .to_xdr(&env, &client.address),
            expected_event.to_xdr(&env, &client.address)
        ],
    );
}

#[test]
fn attest_without_attester_auth_fails() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&admin, &attester);
    let record_hash = BytesN::from_array(&env, &[5u8; 32]);

    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    env.mock_auths(&[]);
    let result = client.try_attest(&attester, &patient, &record_hash);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_accepts_attester_authorization_entry_for_sponsored_submission() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[27u8; 32]);

    // The only authorization supplied is for the attester's exact contract
    // invocation; a relayer/source-account authorization is not required here.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest",
            args: (attester.clone(), record_hash.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let attestation = client.attest(&attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn propose_admin_by_non_admin_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    client.initialize(&admin, &attester_registry_id);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (new_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_propose_admin(&new_admin);
    assert!(result.is_err());
}

#[test]
fn accept_admin_by_wrong_address_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    client.initialize(&admin, &attester_registry_id);
    client.propose_admin(&new_admin);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_accept_admin();
    assert!(result.is_err());
}

#[test]
fn accept_admin_with_no_pending_proposal_fails() {
    let (_env, client, _, _admin) = setup();

    let result = client.try_accept_admin();
    assert_eq!(result, Err(Ok(Error::NoPendingTransfer)));
}

#[test]
fn successful_admin_transfer_flow() {
    let (env, client, _, admin) = setup();

    let new_admin = Address::generate(&env);

    client.propose_admin(&new_admin);

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "propose_admin"),
                    (new_admin.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    client.accept_admin();

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "accept_admin"),
                    ().into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    let expected_event = AdminTransferred {
        previous_admin: admin.clone(),
        new_admin: new_admin.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    let newer_admin = Address::generate(&env);
    client.propose_admin(&newer_admin);

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "propose_admin"),
                    (newer_admin.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (newer_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_propose_admin(&newer_admin);
    assert!(result.is_err());
}

#[test]
fn initialize_rejects_non_contract_address() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    // A non-contract address (just a generated address, never deployed)
    let non_contract = Address::generate(&env);

    let result = client.try_initialize(&admin, &non_contract);
    // Same `Error::InvalidRegistryWiring` variant as
    // `initialize_rejects_unrelated_contract_without_regional_check` below —
    // the contract does not distinguish "not a contract at all" from "a
    // contract, but missing `is_attester`"; both are one generic wiring error.
    assert_eq!(result, Err(Ok(Error::InvalidRegistryWiring)));
}

#[test]
fn initialize_rejects_unrelated_contract_without_regional_check() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    // Use the attestation-registry contract itself as the "attester registry"
    // address — it's a valid deployed contract but does NOT implement
    // the is_attester_for_region interface, so the sanity check should reject it.
    let result = client.try_initialize(&admin, &contract_id);
    // Same `Error::InvalidRegistryWiring` variant as
    // `initialize_rejects_non_contract_address` above — see that test's
    // comment for why the two rejection paths are not distinguished.
    assert_eq!(result, Err(Ok(Error::InvalidRegistryWiring)));
}

#[test]
fn initialize_accepts_real_attester_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);
    attester_registry_client.initialize(&admin);
    attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);

    let result = client.try_initialize(&admin, &attester_registry_id);
    assert_eq!(result, Ok(Ok(())));
    assert_eq!(client.get_attester_registry(), attester_registry_id);
}

fn parse_error_variants(content: &str) -> std::vec::Vec<std::string::String> {
    let mut variants = std::vec::Vec::new();
    if let Some(start_idx) = content.find("pub enum Error") {
        if let Some(block_start) = content[start_idx..].find('{') {
            let block = &content[start_idx + block_start + 1..];
            if let Some(block_end) = block.find('}') {
                let body = &block[..block_end];
                for line in body.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with("//") {
                        continue;
                    }
                    if let Some(first_char) = line.chars().next() {
                        if first_char.is_ascii_alphabetic() {
                            let name: std::string::String = line
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric())
                                .collect();
                            if !name.is_empty() {
                                variants.push(name);
                            }
                        }
                    }
                }
            }
        }
    }
    variants
}

fn parse_contract_events(content: &str) -> std::vec::Vec<std::string::String> {
    let mut events = std::vec::Vec::new();
    let mut event_attribute_seen = false;

    for line in content.lines() {
        let line = line.trim();
        if line == "#[contractevent]" {
            event_attribute_seen = true;
        } else if let (true, Some(declaration)) =
            (event_attribute_seen, line.strip_prefix("pub struct "))
        {
            let name = declaration
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            events.push(name);
            event_attribute_seen = false;
        }
    }

    events
}

fn markdown_section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let body = &content[start + heading.len()..];
    let end = body
        .find("\n## ")
        .map_or(content.len(), |offset| start + heading.len() + offset);
    Some(&content[start..end])
}

#[test]
fn test_error_codes_are_documented() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let doc_path = workspace_root.join("docs").join("error-codes.md");
    let doc_content = std::fs::read_to_string(&doc_path)
        .expect("Failed to read docs/error-codes.md. Make sure it exists.");

    let attester_src_path = workspace_root
        .join("contracts")
        .join("attester-registry")
        .join("src")
        .join("lib.rs");
    let attester_src = std::fs::read_to_string(&attester_src_path)
        .expect("Failed to read attester-registry lib.rs");

    let attestation_src_path = workspace_root
        .join("contracts")
        .join("attestation-registry")
        .join("src")
        .join("lib.rs");
    let attestation_src = std::fs::read_to_string(&attestation_src_path)
        .expect("Failed to read attestation-registry lib.rs");
    let multisig_src_path = workspace_root
        .join("contracts")
        .join("multisig-account")
        .join("src")
        .join("lib.rs");
    let multisig_src = std::fs::read_to_string(&multisig_src_path)
        .expect("Failed to read multisig-account lib.rs");

    for (contract, source) in [
        ("attester-registry", attester_src.as_str()),
        ("attestation-registry", attestation_src.as_str()),
        ("multisig-account", multisig_src.as_str()),
    ] {
        let variants = parse_error_variants(source);
        assert!(
            !variants.is_empty(),
            "Could not find any Error variants in {contract}"
        );

        let heading = std::format!("## `{contract}`");
        let section = markdown_section(&doc_content, &heading)
            .unwrap_or_else(|| panic!("Missing '{heading}' section in docs/error-codes.md"));
        for variant in variants {
            assert!(
                section.contains(&std::format!("`{variant}`")),
                "Error variant '{variant}' is not documented under '{heading}' in docs/error-codes.md"
            );
        }
    }
}

#[test]
fn test_contract_events_are_documented() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let doc_path = workspace_root
        .join("docs")
        .join("architecture")
        .join("event-indexing.md");
    let doc_content = std::fs::read_to_string(&doc_path)
        .expect("Failed to read docs/architecture/event-indexing.md. Make sure it exists.");

    for contract in ["attester-registry", "attestation-registry"] {
        let source_path = workspace_root
            .join("contracts")
            .join(contract)
            .join("src")
            .join("lib.rs");
        let source = std::fs::read_to_string(&source_path)
            .unwrap_or_else(|_| panic!("Failed to read {contract} lib.rs"));
        let events = parse_contract_events(&source);
        assert!(
            !events.is_empty(),
            "Could not find any contract events in {contract}"
        );

        for event in events {
            assert!(
                doc_content.contains(&std::format!("`{event}`")),
                "Event '{event}' from {contract} is not documented in docs/architecture/event-indexing.md"
            );
        }
    }
}

#[test]
fn test_initialize_auth_matrix() {
    struct TestCase {
        name: &'static str,
        auth_role: &'static str, // "admin", "wrong_user", "none", "attester"
        expected_result: Result<
            Result<(), soroban_sdk::ConversionError>,
            Result<Error, soroban_sdk::InvokeError>,
        >,
    }

    let cases = std::vec![
        TestCase {
            name: "Right Caller (Admin)",
            auth_role: "admin",
            expected_result: Ok(Ok(())),
        },
        TestCase {
            name: "Wrong Caller (Wrong User)",
            auth_role: "wrong_user",
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "No Auth Provided",
            auth_role: "none",
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "Role Confusion (Attester)",
            auth_role: "attester",
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
    ];

    for case in cases {
        let env = Env::default();
        let contract_id = env.register(AttestationRegistry, ());
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let wrong_user = Address::generate(&env);
        let attester = Address::generate(&env);

        // Must be a real attester-registry contract, not a bare generated
        // address: `initialize` does a best-effort `is_attester` interface
        // check on it before the admin auth even matters for the happy path.
        let attester_registry = env.register(attester_registry::AttesterRegistry, ());
        let attester_registry_client =
            attester_registry::AttesterRegistryClient::new(&env, &attester_registry);
        env.mock_all_auths();
        attester_registry_client.initialize(&admin);
        attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);

        let auth_address = match case.auth_role {
            "admin" => Some(admin.clone()),
            "wrong_user" => Some(wrong_user.clone()),
            "attester" => Some(attester.clone()),
            _ => None,
        };

        if let Some(addr) = auth_address {
            env.mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &addr,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &client.address,
                    fn_name: "initialize",
                    args: (admin.clone(), attester_registry.clone()).into_val(&env),
                    sub_invokes: &[],
                },
            }]);
        } else {
            env.mock_auths(&[]);
        }

        let result = client.try_initialize(&admin, &attester_registry);
        assert_eq!(
            result, case.expected_result,
            "Failed case '{}': expected {:?}, got {:?}",
            case.name, case.expected_result, result
        );
    }
}

#[test]
fn test_attest_auth_matrix() {
    struct TestCase {
        name: &'static str,
        call_role: &'static str, // "attester", "wrong_user", "admin"
        auth_role: &'static str, // "attester", "wrong_user", "admin", "none"
        allowlisted: bool,
        expected_result: Result<
            Result<Attestation, soroban_sdk::ConversionError>,
            Result<Error, soroban_sdk::InvokeError>,
        >,
    }

    let cases = std::vec![
        TestCase {
            name: "Right Caller (Attester)",
            call_role: "attester",
            auth_role: "attester",
            allowlisted: true,
            expected_result: Ok(Ok(Attestation {
                attester: Address::generate(&Env::default()), // will be overwritten in comparison/check
                timestamp: 0,
                commitment_version: 0,
            })),
        },
        TestCase {
            name: "Wrong Caller (not allowlisted)",
            call_role: "wrong_user",
            auth_role: "wrong_user",
            allowlisted: false,
            expected_result: Err(Ok(Error::AttesterNotAllowlisted)),
        },
        TestCase {
            name: "Wrong Caller (wrong auth)",
            call_role: "attester",
            auth_role: "wrong_user",
            allowlisted: true,
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "No Auth Provided",
            call_role: "attester",
            auth_role: "none",
            allowlisted: true,
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "Role Confusion (Admin)",
            call_role: "admin",
            auth_role: "admin",
            allowlisted: false,
            expected_result: Err(Ok(Error::AttesterNotAllowlisted)),
        },
    ];

    for case in cases {
        let env = Env::default();
        let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
        let attester_registry_client =
            attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

        let contract_id = env.register(AttestationRegistry, ());
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let wrong_user = Address::generate(&env);
        let attester = Address::generate(&env);
        let patient = Address::generate(&env);
        let record_hash = BytesN::from_array(&env, &[99u8; 32]);

        // Setup attester registry allowlist
        env.mock_all_auths();
        attester_registry_client.initialize(&admin);
        attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);
        client.initialize(&admin, &attester_registry_id);

        if case.allowlisted {
            attester_registry_client.add_attester(&attester);
            let expires_at = env.ledger().timestamp() + 10_000;
            client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
        }

        // Determine which addresses are used for call vs auth
        let call_address = match case.call_role {
            "attester" => attester.clone(),
            "wrong_user" => wrong_user.clone(),
            "admin" => admin.clone(),
            _ => panic!("Unknown call role"),
        };

        let auth_address = match case.auth_role {
            "attester" => Some(attester.clone()),
            "wrong_user" => Some(wrong_user.clone()),
            "admin" => Some(admin.clone()),
            _ => None,
        };
        let region = test_region(&env);

        if let Some(addr) = auth_address {
            env.mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &addr,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &client.address,
                    fn_name: "attest",
                    args: (call_address.clone(), patient.clone(), record_hash.clone())
                        .into_val(&env),
                    sub_invokes: &[],
                },
            }]);
        } else {
            env.mock_auths(&[]);
        }

        let result = client.try_attest(&call_address, &patient, &record_hash);

        // Check matching expected results
        match (&result, &case.expected_result) {
            (Ok(Ok(attestation)), Ok(Ok(_))) => {
                assert_eq!(attestation.attester, call_address);
                assert_eq!(
                    client.get_attestation(&record_hash),
                    Some(attestation.clone())
                );
            }
            (Err(Ok(err)), Err(Ok(expected_err))) => {
                assert_eq!(err, expected_err);
                assert_eq!(client.get_attestation(&record_hash), None);
            }
            (
                Err(Err(soroban_sdk::InvokeError::Abort)),
                Err(Err(soroban_sdk::InvokeError::Abort)),
            ) => {
                assert_eq!(client.get_attestation(&record_hash), None);
            }
            _ => panic!(
                "Failed case '{}': expected {:?}, got {:?}",
                case.name, case.expected_result, result
            ),
        }
    }
}

#[test]
fn set_attester_registry_by_admin_succeeds() {
    let (env, client, attester_registry, admin) = setup();

    let new_registry = env.register(attester_registry::AttesterRegistry, ());
    let new_registry_client = attester_registry::AttesterRegistryClient::new(&env, &new_registry);
    new_registry_client.initialize(&admin);
    assert_eq!(client.get_attester_registry(), attester_registry.address);

    client.set_attester_registry(&new_registry);

    // Check event was emitted before any other call clears it
    let expected_event = AttesterRegistryRepointed {
        previous: attester_registry.address.clone(),
        new: new_registry.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "set_attester_registry"),
                    (new_registry.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    assert_eq!(client.get_attester_registry(), new_registry);
}

#[test]
fn set_attester_registry_rejects_incompatible_contract() {
    let (env, client, attester_registry, _admin) = setup();
    let incompatible = env.register(AttestationRegistry, ());

    assert_eq!(
        client.try_set_attester_registry(&incompatible),
        Err(Ok(Error::InvalidRegistryWiring))
    );
    assert_eq!(client.get_attester_registry(), attester_registry.address);
}

#[test]
fn set_attester_registry_by_non_admin_fails() {
    let (env, client, attester_registry, _admin) = setup();
    let malicious = Address::generate(&env);
    let new_registry = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_attester_registry",
            args: (new_registry.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_set_attester_registry(&new_registry);
    assert!(result.is_err());
    // Registry should be unchanged
    assert_eq!(client.get_attester_registry(), attester_registry.address);
}

#[test]
fn set_attester_registry_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let new_registry = Address::generate(&env);

    let result = client.try_set_attester_registry(&new_registry);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn revoke_attestation_happy_path() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&admin, &attester);

    let record_hash = BytesN::from_array(&env, &[11u8; 32]);
    attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.get_attestation(&record_hash).is_some());

    client.revoke_attestation(&admin, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(
        client.get_attestation_status(&record_hash),
        AttestationStatus::Revoked
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester),
        AttesterAttestationStatus::Revoked
    );
    assert_eq!(client.get_attestation_history(&record_hash).len(), 1);

    let replacement = client.attest(&attester, &record_hash);
    assert_eq!(client.get_attestation(&record_hash), Some(replacement));
    assert_eq!(
        client.get_attestation_status(&record_hash),
        AttestationStatus::Verified
    );
}

#[test]
fn revoke_attestation_without_admin_auth_fails() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let malicious = Address::generate(&env);
    attester_registry.add_attester(&_admin, &attester);

    let record_hash = BytesN::from_array(&env, &[12u8; 32]);
    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "revoke_attestation",
            args: (_admin.clone(), record_hash.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_revoke_attestation(&_admin, &record_hash);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn revoke_attestation_for_unknown_hash_returns_attestation_not_found() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[13u8; 32]);

    let result = client.try_revoke_attestation(&record_hash);
    assert_eq!(result, Err(Ok(Error::AttestationNotFound)));
}

#[test]
fn revoke_attestation_preserves_get_attestation_history() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    let attester_c = Address::generate(&env);
    attester_registry.add_attester(&_admin, &attester_a);
    attester_registry.add_attester(&_admin, &attester_b);
    attester_registry.add_attester(&_admin, &attester_c);

    let record_hash = BytesN::from_array(&env, &[14u8; 32]);
    let first = attest_with_consent(&env, &client, &attester_a, &record_hash);
    let second = attest_with_consent(&env, &client, &attester_b, &record_hash);
    let third = attest_with_consent(&env, &client, &attester_c, &record_hash);

    let history_before = client.get_attestation_history(&record_hash);
    assert_eq!(history_before.len(), 3);
    assert_eq!(history_before.get(0), Some(first));
    assert_eq!(history_before.get(1), Some(second));
    assert_eq!(history_before.get(2), Some(third.clone()));

    assert_eq!(client.get_attestation(&record_hash), Some(third.clone()));

    client.revoke_attestation(&_admin, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), None);
    let history_after = client.get_attestation_history(&record_hash);
    assert_eq!(history_after.len(), 3);
    assert_eq!(
        client.get_attestation_status(&record_hash),
        AttestationStatus::Revoked
    );
}

#[test]
fn get_interface_reports_kind_versions_and_features() {
    let (env, client, attester_registry, _admin) = setup();

    let info = client.get_interface();
    assert_eq!(
        info.contract_kind,
        Symbol::new(&env, "lafiya_attestation_registry")
    );
    assert_eq!(info.interface_version, INTERFACE_VERSION);
    assert_eq!(info.schema_version, 1);
    assert_eq!(info.event_version, EVENT_VERSION);
    assert!(info.features.contains(Symbol::new(&env, "revocation")));

    // Each contract reports its own kind, so wiring can be sanity-checked.
    assert_ne!(
        attester_registry.get_interface().contract_kind,
        info.contract_kind
    );
}
fn advance_ledgers(env: &Env, n: u32) {
    use soroban_sdk::testutils::Ledger as _;
    env.ledger().with_mut(|l| l.sequence_number += n);
}

fn rate_limited_setup(
    max_per_window: u32,
    window_ledgers: u32,
) -> (Env, AttestationRegistryClient<'static>, Address) {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    client.set_attestation_rate_limit(&max_per_window, &window_ledgers);
    (env, client, attester)
}

fn hash(env: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(env, &[n; 32])
}

#[test]
fn rate_limit_disabled_by_default() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    assert_eq!(client.get_attestation_rate_limit(), None);
    for i in 0..20 {
        client.attest(&attester, &hash(&env, i));
    }
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);
}

#[test]
fn rate_limit_under_and_at_limit_succeeds() {
    let (env, client, attester) = rate_limited_setup(3, 100);

    client.attest(&attester, &hash(&env, 1));
    client.attest(&attester, &hash(&env, 2));
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);

    client.attest(&attester, &hash(&env, 3));
    let start = env.ledger().sequence();
    assert_eq!(
        client.get_rate_limit_retry_after(&attester),
        Some(start + 100)
    );
}

#[test]
fn rate_limit_hit_event_emitted_when_window_fills() {
    let (env, client, attester) = rate_limited_setup(1, 100);
    let start = env.ledger().sequence();

    let record_hash = hash(&env, 1);
    let attestation = client.attest(&attester, &record_hash);

    let hit = RateLimitHit {
        attester: attester.clone(),
        retry_after_ledger: start + 100,
    };
    let recorded = AttestationRecorded {
        record_hash,
        attester,
        timestamp: attestation.timestamp,
    };
    assert_eq!(
        env.events().all(),
        std::vec![
            hit.to_xdr(&env, &client.address),
            recorded.to_xdr(&env, &client.address)
        ],
    );
}

#[test]
fn rate_limit_over_limit_fails() {
    let (env, client, attester) = rate_limited_setup(2, 100);

    client.attest(&attester, &hash(&env, 1));
    client.attest(&attester, &hash(&env, 2));
    assert_eq!(
        client.try_attest(&attester, &hash(&env, 3)),
        Err(Ok(Error::RateLimited))
    );
    assert_eq!(client.get_attestation(&hash(&env, 3)), None);
}

#[test]
fn rate_limit_window_rollover_resets_count() {
    let (env, client, attester) = rate_limited_setup(1, 100);

    client.attest(&attester, &hash(&env, 1));
    advance_ledgers(&env, 99);
    assert_eq!(
        client.try_attest(&attester, &hash(&env, 2)),
        Err(Ok(Error::RateLimited))
    );

    advance_ledgers(&env, 1);
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);
    client.attest(&attester, &hash(&env, 2));
}

#[test]
fn rate_limit_is_per_attester() {
    let (env, client, attester_registry, _admin) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    attester_registry.add_attester(&a);
    attester_registry.add_attester(&b);
    client.set_attestation_rate_limit(&1, &100);

    client.attest(&a, &hash(&env, 1));
    client.attest(&b, &hash(&env, 2));
    assert_eq!(
        client.try_attest(&a, &hash(&env, 3)),
        Err(Ok(Error::RateLimited))
    );
}

#[test]
fn rate_limit_per_attester_override() {
    let (env, client, attester) = rate_limited_setup(1, 100);
    client.set_attester_rate_limit(&attester, &3);

    for i in 0..3 {
        client.attest(&attester, &hash(&env, i));
    }
    assert_eq!(
        client.try_attest(&attester, &hash(&env, 9)),
        Err(Ok(Error::RateLimited))
    );

    client.remove_attester_rate_limit(&attester);
    advance_ledgers(&env, 100);
    client.attest(&attester, &hash(&env, 10));
    assert_eq!(
        client.try_attest(&attester, &hash(&env, 11)),
        Err(Ok(Error::RateLimited))
    );
}

#[test]
fn rate_limit_fails_open_after_temporary_entry_expiry() {
    let (env, client, attester) = rate_limited_setup(1, 100);
    client.attest(&attester, &hash(&env, 1));

    // Simulate the temporary `RateWindow` entry being archived before the
    // window ends: the attester starts a fresh window.
    env.as_contract(&client.address, || {
        env.storage()
            .temporary()
            .remove(&DataKey::RateWindow(attester.clone()));
    });

    client.attest(&attester, &hash(&env, 2));
    assert_eq!(
        client.try_attest(&attester, &hash(&env, 3)),
        Err(Ok(Error::RateLimited))
    );
}

#[test]
fn rate_limit_zero_disables_and_invalid_window_rejected() {
    let (env, client, attester) = rate_limited_setup(1, 100);

    assert_eq!(
        client.try_set_attestation_rate_limit(&5, &0),
        Err(Ok(Error::InvalidRateLimit))
    );
    assert_eq!(
        client.try_set_attestation_rate_limit(&5, &(MAX_RATE_WINDOW_LEDGERS + 1)),
        Err(Ok(Error::InvalidRateLimit))
    );

    client.set_attestation_rate_limit(&0, &0);
    assert_eq!(client.get_attestation_rate_limit(), None);
    for i in 0..5 {
        client.attest(&attester, &hash(&env, i));
    }
}

#[test]
fn set_attestation_rate_limit_requires_admin_auth() {
    let env = Env::default();
    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    env.mock_all_auths();
    attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id).initialize(&admin);
    client.initialize(&admin, &attester_registry_id);

    env.set_auths(&[]);
    assert!(client.try_set_attestation_rate_limit(&1, &100).is_err());
}
