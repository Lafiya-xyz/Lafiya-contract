extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{BytesN, Env, Event, IntoVal};

type ArClient = attester_registry::AttesterRegistryClient<'static>;

/// Registers an attester-registry and an attestation-registry wired to it.
/// The returned admin holds every role in both contracts.
fn setup() -> (Env, AttestationRegistryClient<'static>, ArClient, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);
    attester_registry_client.grant_role(&attester_registry::Role::Registrar, &admin);
    attester_registry_client.grant_role(&attester_registry::Role::Guardian, &admin);

    let contract_id = env.register(
        AttestationRegistry,
        (admin.clone(), attester_registry_id.clone()),
    );
    let client = AttestationRegistryClient::new(&env, &contract_id);
    client.grant_role(&Role::Guardian, &admin);
    client.grant_role(&Role::Revoker, &admin);

    (env, client, attester_registry_client, admin)
}

fn enroll(env: &Env, registry: &ArClient, admin: &Address) -> Address {
    let attester = Address::generate(env);
    registry.add_attester(admin, &attester);
    attester
}

fn hash(env: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(env, &[n; 32])
}

/// Grants consent from a fresh patient (valid for 10_000s) and attests.
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

fn advance_ledgers(env: &Env, n: u32) {
    env.ledger().with_mut(|l| l.sequence_number += n);
}

fn recorded_event(
    env: &Env,
    record_hash: &BytesN<32>,
    attestation: &Attestation,
    expires_at: u64,
    sequence: u64,
    evicted_sequence: Option<u64>,
) -> AttestationRecorded {
    AttestationRecorded {
        record_hash: record_hash.clone(),
        attester: attestation.attester.clone(),
        timestamp: attestation.timestamp,
        expires_at,
        commitment_version: attestation.commitment_version,
        sequence,
        evicted_sequence,
        contract_kind: Symbol::new(env, "attestation_registry"),
        schema_version: EVENT_SCHEMA_VERSION,
    }
}

// ───────────────────────── Configuration ─────────────────────────

#[test]
fn configuration_getters_return_initialized_addresses() {
    let (_env, client, attester_registry, admin) = setup();

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_attester_registry(), attester_registry.address);
    assert_eq!(client.get_schema_version(), SCHEMA_VERSION);
    assert!(client.has_role(&Role::Guardian, &admin));
    assert!(client.has_role(&Role::Revoker, &admin));
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
fn get_interface_reports_kind_versions_and_features() {
    let (_env, client, attester_registry, _admin) = setup();
    let info = client.get_interface();

    assert_eq!(
        info.contract_kind,
        Symbol::new(&_env, "lafiya_attestation_registry")
    );
    assert_eq!(info.interface_version, INTERFACE_VERSION);
    assert_eq!(info.schema_version, client.get_schema_version());
    assert_eq!(info.event_version, EVENT_VERSION);
    assert_eq!(info.features.len(), FEATURES.len() as u32);
    assert!(info.features.contains(Symbol::new(&_env, "revocation")));
    assert_ne!(
        attester_registry.get_interface().contract_kind,
        info.contract_kind
    );
}

// ───────────────────────── Wiring ─────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_non_contract_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let non_contract = Address::generate(&env);
    env.register(AttestationRegistry, (admin, non_contract));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_contract_without_regional_check() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    // A deployed contract that does not implement `is_attester_for_region`.
    let unrelated = env.register(
        AttestationRegistry,
        (admin.clone(), {
            env.register(attester_registry::AttesterRegistry, (admin.clone(),))
        }),
    );
    env.register(AttestationRegistry, (admin, unrelated));
}

#[test]
fn set_attester_registry_by_admin_succeeds() {
    let (env, client, attester_registry, admin) = setup();
    let new_registry = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    assert_eq!(client.get_attester_registry(), attester_registry.address);

    client.set_attester_registry(&new_registry);

    let expected_event = AttesterRegistryRepointed {
        previous: attester_registry.address.clone(),
        new: new_registry.clone(),
        contract_kind: Symbol::new(&env, "attestation_registry"),
        schema_version: EVENT_SCHEMA_VERSION,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
    assert_eq!(client.get_attester_registry(), new_registry);
}

#[test]
fn set_attester_registry_rejects_incompatible_contract() {
    let (env, client, attester_registry, _admin) = setup();
    // The attestation registry itself has no `is_attester_for_region`.
    assert_eq!(
        client.try_set_attester_registry(&client.address),
        Err(Ok(Error::InvalidRegistryWiring))
    );
    let non_contract = Address::generate(&env);
    assert_eq!(
        client.try_set_attester_registry(&non_contract),
        Err(Ok(Error::InvalidRegistryWiring))
    );
    assert_eq!(client.get_attester_registry(), attester_registry.address);
}

#[test]
fn set_attester_registry_by_non_admin_fails() {
    let (env, client, _attester_registry, admin) = setup();
    let new_registry = env.register(attester_registry::AttesterRegistry, (admin,));
    let malicious = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_attester_registry",
            args: (new_registry.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_attester_registry(&new_registry).is_err());
}

// ───────────────────────── Consent and attest ─────────────────────────

#[test]
fn attest_by_allowlisted_attester_succeeds() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 7);

    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(attestation.commitment_version, 0);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn attest_requires_matching_patient_consent() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 71);

    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::PatientConsentRequired))
    );

    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    let other_patient = Address::generate(&env);
    assert_eq!(
        client.try_attest(&attester, &other_patient, &record_hash),
        Err(Ok(Error::PatientConsentRequired))
    );

    let attestation = client.attest(&attester, &patient, &record_hash);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));

    // The grant is single-use.
    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::PatientConsentRequired))
    );
}

#[test]
fn expired_consent_grant_is_rejected() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 73);

    let expires_at = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + CONSENT_VALIDITY_SECONDS);

    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::PatientConsentExpired))
    );
}

#[test]
fn patient_consent_requires_patient_authorization() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 72);

    env.mock_auths(&[]);
    let expires_at = env.ledger().timestamp() + 10_000;
    let result = client.try_consent_attestation(&patient, &attester, &record_hash, &expires_at);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn patient_cannot_grant_already_expired_attestation() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 16);

    let expires_at = env.ledger().timestamp();
    assert_eq!(
        client.try_consent_attestation(&patient, &attester, &record_hash, &expires_at),
        Err(Ok(Error::InvalidAttestationExpiry))
    );
}

#[test]
fn attest_by_non_allowlisted_attester_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 1);

    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::AttesterNotAllowlisted))
    );
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_without_attester_auth_fails() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 5);
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);

    env.mock_auths(&[]);
    assert!(client
        .try_attest(&attester, &patient, &record_hash)
        .is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_accepts_attester_authorization_entry_for_sponsored_submission() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 27);
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);

    // The only authorization supplied is for the attester's exact contract
    // invocation; a relayer/source-account authorization is not required.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest",
            args: (attester.clone(), patient.clone(), record_hash.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let attestation = client.attest(&attester, &patient, &record_hash);
    assert_eq!(attestation.attester, attester);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn attest_before_initialize_fails() {
    // A contract registered without constructor arguments has no configuration.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let registry = env.register(attester_registry::AttesterRegistry, (admin,));
    let contract_id = env.register(AttestationRegistry, (Address::generate(&env), registry));
    let client = AttestationRegistryClient::new(&env, &contract_id);
    env.as_contract(&client.address, || {
        env.storage().instance().remove(&DataKey::AttesterRegistry);
    });

    let result = client.try_attest(
        &Address::generate(&env),
        &Address::generate(&env),
        &hash(&env, 2),
    );
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn attest_returns_registry_unavailable_when_registry_call_traps() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 12);
    let broken_registry = Address::generate(&env);
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &broken_registry);
    });

    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::AttesterRegistryUnavailable))
    );
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_versioned_records_commitment_version() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 28);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);

    let attestation = client.attest_versioned(&attester, &patient, &record_hash, &1);
    assert_eq!(attestation.commitment_version, 1);

    assert_eq!(
        client.try_attest_versioned(&attester, &patient, &record_hash, &256),
        Err(Ok(Error::InvalidCommitmentVersion))
    );
}

#[test]
fn attest_emits_event() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 4);

    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    let expected = recorded_event(
        &env,
        &record_hash,
        &attestation,
        env.ledger().timestamp() + 10_000,
        1,
        None,
    );
    assert_eq!(
        env.events().all(),
        std::vec![expected.to_xdr(&env, &client.address)],
    );
}

// ───────────────────────── Record versions and batches ─────────────────────────

#[test]
fn attest_version_links_successive_record_versions() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let v1 = hash(&env, 30);
    let v2 = hash(&env, 31);
    attest_with_consent(&env, &client, &attester, &v1);

    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &v2, &expires_at);
    client.attest_version(&attester, &patient, &v2, &v1);

    assert_eq!(client.get_previous_record_hash(&v2), Some(v1.clone()));
    assert_eq!(client.get_next_record_hash(&v1), Some(v2.clone()));
}

#[test]
fn attest_version_rejects_unknown_or_self_reference() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 32);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);

    assert_eq!(
        client.try_attest_version(&attester, &patient, &record_hash, &record_hash),
        Err(Ok(Error::InvalidRecordVersion))
    );
    assert_eq!(
        client.try_attest_version(&attester, &patient, &record_hash, &hash(&env, 99)),
        Err(Ok(Error::InvalidRecordVersion))
    );
}

#[test]
fn batch_attest_records_each_request_and_enforces_limit() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let mut requests = Vec::new(&env);
    for i in 0..3u8 {
        let patient = Address::generate(&env);
        let record_hash = hash(&env, 40 + i);
        let expires_at = env.ledger().timestamp() + 10_000;
        client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
        requests.push_back(AttestationRequest {
            attester: attester.clone(),
            patient,
            record_hash,
            previous_record_hash: None,
        });
    }

    let results = client.batch_attest(&requests);
    assert_eq!(results.len(), 3);
    for i in 0..3u8 {
        assert!(client.get_attestation(&hash(&env, 40 + i)).is_some());
    }

    let mut too_many = Vec::new(&env);
    for i in 0..=BATCH_LIMIT {
        too_many.push_back(AttestationRequest {
            attester: attester.clone(),
            patient: Address::generate(&env),
            record_hash: hash(&env, i as u8),
            previous_record_hash: None,
        });
    }
    assert_eq!(
        client.try_batch_attest(&too_many),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn anchor_batch_stores_one_root_and_returns_metadata() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let root = hash(&env, 21);

    let batch = client.anchor_batch(&attester, &root, &3);

    assert_eq!(batch.attester, attester);
    assert_eq!(batch.leaf_count, 3);
    assert_eq!(client.get_attestation_batch(&root), Some(batch));
}

#[test]
fn anchor_batch_rejects_empty_batch() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let root = hash(&env, 22);

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
    let root = hash(&env, 23);

    assert_eq!(
        client.try_anchor_batch(&attester, &root, &2),
        Err(Ok(Error::AttesterNotAllowlisted))
    );
    assert_eq!(client.get_attestation_batch(&root), None);
}

#[test]
fn anchor_batch_rejects_reused_root() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let root = hash(&env, 24);
    client.anchor_batch(&attester, &root, &2);

    assert_eq!(
        client.try_anchor_batch(&attester, &root, &2),
        Err(Ok(Error::BatchAlreadyAnchored))
    );
}

// ───────────────────────── Lookups ─────────────────────────

#[test]
fn get_attestation_returns_none_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = hash(&env, 9);
    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(client.get_attestation_status(&record_hash), None);
    assert_eq!(
        client.get_record_status(&record_hash),
        RecordStatus::NeverAttested
    );
}

#[test]
fn get_attestations_returns_results_in_input_order_including_misses() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);

    let first_hash = hash(&env, 15);
    let missing_hash = hash(&env, 16);
    let last_hash = hash(&env, 17);
    let first = attest_with_consent(&env, &client, &attester, &first_hash);
    let last = attest_with_consent(&env, &client, &attester, &last_hash);

    let hashes = Vec::from_array(&env, [first_hash, missing_hash, last_hash]);
    let results = client.get_attestations(&hashes);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0), Some(Some(first)));
    assert_eq!(results.get(1), Some(None));
    assert_eq!(results.get(2), Some(Some(last)));
}

#[test]
fn is_verified_applies_age_and_current_attester_status() {
    let (env, client, attester_registry, admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = hash(&env, 13);

    assert!(!client.is_verified(&record_hash));
    assert_eq!(
        client.get_max_attestation_age(),
        DEFAULT_MAX_ATTESTATION_AGE
    );

    attester_registry.add_attester(&admin, &attester);
    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.is_verified(&record_hash));

    attester_registry.suspend_attester(&admin, &attester);
    assert!(!client.is_verified(&record_hash));
    attester_registry.reinstate_attester(&admin, &attester);
    assert!(client.is_verified(&record_hash));

    client.set_max_attestation_age(&0);
    assert_eq!(client.get_max_attestation_age(), 0);
    env.ledger()
        .set_timestamp(attestation.timestamp.saturating_add(1));
    assert!(!client.is_verified(&record_hash));

    client.set_max_attestation_age(&DEFAULT_MAX_ATTESTATION_AGE);
    env.ledger().set_timestamp(attestation.timestamp);
    attester_registry.remove_attester(&admin, &attester);
    assert!(!client.is_verified(&record_hash));
}

#[test]
fn is_verified_is_false_after_revocation() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 18);
    attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.is_verified(&record_hash));

    client.revoke_attestation(&admin, &record_hash, &Symbol::new(&env, "compromised"));
    assert!(!client.is_verified(&record_hash));
}

#[test]
fn is_verified_reports_broken_registry_call() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 14);
    attest_with_consent(&env, &client, &attester, &record_hash);
    let broken_registry = Address::generate(&env);
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &broken_registry);
    });

    assert_eq!(
        client.try_is_verified(&record_hash),
        Err(Ok(Error::AttesterRegistryUnavailable))
    );
}

#[test]
fn is_attestation_trusted_follows_attester_trust_cutoff() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 19);
    env.ledger().set_timestamp(100);
    attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.is_attestation_trusted(&record_hash));

    // Removal stamps a trust cutoff at the removal time.
    env.ledger().set_timestamp(200);
    attester_registry.remove_attester(&admin, &attester);
    assert!(client.is_attestation_trusted(&record_hash));

    // Re-enroll, attest later, then remove at an earlier cutoff is impossible;
    // an attestation made at/after the cutoff is not trusted.
    attester_registry.add_attester(&admin, &attester);
    env.ledger().set_timestamp(300);
    let later = hash(&env, 20);
    attest_with_consent(&env, &client, &attester, &later);
    attester_registry.suspend_attester(&admin, &attester);
    assert!(!client.is_attestation_trusted(&later));
}

// ───────────────────────── Retention ─────────────────────────

#[test]
fn distinct_attesters_are_kept_in_history() {
    let (env, client, attester_registry, admin) = setup();
    let attester_a = enroll(&env, &attester_registry, &admin);
    let attester_b = enroll(&env, &attester_registry, &admin);

    let record_hash = hash(&env, 3);
    let first = attest_with_consent(&env, &client, &attester_a, &record_hash);
    let second = attest_with_consent(&env, &client, &attester_b, &record_hash);

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0), Some(first));
    assert_eq!(history.get(1), Some(second));
}

#[test]
fn repeated_attestations_refresh_one_slot_without_evicting_others() {
    let (env, client, attester_registry, admin) = setup();
    let clinician = enroll(&env, &attester_registry, &admin);
    let repeated = enroll(&env, &attester_registry, &admin);

    let record_hash = hash(&env, 12);
    let clinician_attestation = attest_with_consent(&env, &client, &clinician, &record_hash);
    for _ in 0..(MAX_HISTORY + 5) {
        attest_with_consent(&env, &client, &repeated, &record_hash);
    }

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0), Some(clinician_attestation));
    assert_eq!(history.get(1).map(|a| a.attester), Some(repeated));
}

#[test]
fn get_attestation_history_returns_empty_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    assert_eq!(client.get_attestation_history(&hash(&env, 8)).len(), 0);
}

#[test]
fn attestation_history_is_bounded_fifo() {
    let (env, client, attester_registry, admin) = setup();
    let record_hash = hash(&env, 11);

    let mut attestations = std::vec![];
    for _ in 0..(MAX_HISTORY + 1) {
        let attester = enroll(&env, &attester_registry, &admin);
        attestations.push(attest_with_consent(&env, &client, &attester, &record_hash));
    }

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len() as u64, MAX_HISTORY);
    for i in 0..MAX_HISTORY as usize {
        assert_eq!(history.get(i as u32), Some(attestations[i + 1].clone()));
    }
}

#[test]
fn attest_event_reports_sequence_and_fifo_eviction() {
    let (env, client, attester_registry, admin) = setup();
    let record_hash = hash(&env, 25);

    let mut last = None;
    for _ in 0..(MAX_HISTORY + 1) {
        let attester = enroll(&env, &attester_registry, &admin);
        last = Some(attest_with_consent(&env, &client, &attester, &record_hash));
    }

    let attestation = last.unwrap();
    let expected = recorded_event(
        &env,
        &record_hash,
        &attestation,
        env.ledger().timestamp() + 10_000,
        MAX_HISTORY + 1,
        Some(1),
    );
    assert_eq!(
        env.events().all().events().last(),
        Some(&expected.to_xdr(&env, &client.address)),
    );
}

// ───────────────────────── Withdraw / revoke / expiry ─────────────────────────

#[test]
fn attester_can_withdraw_only_their_own_attestation() {
    let (env, client, attester_registry, admin) = setup();
    let attester_a = enroll(&env, &attester_registry, &admin);
    let attester_b = enroll(&env, &attester_registry, &admin);

    let record_hash = hash(&env, 42);
    let first = attest_with_consent(&env, &client, &attester_a, &record_hash);
    let second = attest_with_consent(&env, &client, &attester_b, &record_hash);

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
        client.get_record_status(&record_hash),
        RecordStatus::Verified
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester_a),
        AttesterAttestationStatus::Withdrawn
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester_b),
        AttesterAttestationStatus::Active
    );
    // The attestation is retained as an immutable historical record.
    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0), Some(first));
}

#[test]
fn attester_cannot_withdraw_another_attesters_attestation() {
    let (env, client, attester_registry, admin) = setup();
    let owner = enroll(&env, &attester_registry, &admin);
    let other = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 43);
    let attestation = attest_with_consent(&env, &client, &owner, &record_hash);

    assert_eq!(
        client.try_withdraw_attestation(&other, &record_hash),
        Err(Ok(Error::AttestationNotOwned))
    );
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn withdrawing_the_only_attestation_reports_withdrawn_and_reattest_reactivates() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 45);
    attest_with_consent(&env, &client, &attester, &record_hash);
    client.withdraw_attestation(&attester, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(client.get_attestation_history(&record_hash).len(), 1);
    assert_eq!(
        client.get_record_status(&record_hash),
        RecordStatus::Withdrawn
    );
    assert_eq!(
        client.try_withdraw_attestation(&attester, &record_hash),
        Err(Ok(Error::AttestationNotOwned))
    );

    attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.get_attestation(&record_hash).is_some());
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester),
        AttesterAttestationStatus::Active
    );
}

#[test]
fn attester_withdrawal_requires_the_attesters_authorization() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 44);
    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    env.mock_auths(&[]);
    assert!(client
        .try_withdraw_attestation(&attester, &record_hash)
        .is_err());
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn revoke_attestation_happy_path() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 50);
    attest_with_consent(&env, &client, &attester, &record_hash);
    let reason = Symbol::new(&env, "compromised");

    client.revoke_attestation(&admin, &record_hash, &reason);

    let expected_event = AttestationRevoked {
        record_hash: record_hash.clone(),
        by: admin.clone(),
        removed_count: 1,
        reason,
        contract_kind: Symbol::new(&env, "attestation_registry"),
        schema_version: EVENT_SCHEMA_VERSION,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)]
    );
    assert_eq!(client.get_attestation(&record_hash), None);
    assert_eq!(
        client.get_record_status(&record_hash),
        RecordStatus::Revoked
    );
    assert_eq!(
        client.get_attester_attestation_status(&record_hash, &attester),
        AttesterAttestationStatus::Revoked
    );
}

#[test]
fn revoke_attestation_preserves_history_and_allows_fresh_attestation() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 51);
    let original = attest_with_consent(&env, &client, &attester, &record_hash);
    client.revoke_attestation(&admin, &record_hash, &Symbol::new(&env, "audit"));

    assert_eq!(client.get_attestation_history(&record_hash).len(), 1);
    assert_eq!(
        client.get_attestation_history(&record_hash).get(0),
        Some(original)
    );

    // A fresh attestation after revocation is active again and does not
    // resurrect the revoked slot.
    attest_with_consent(&env, &client, &attester, &record_hash);
    assert!(client.get_attestation(&record_hash).is_some());
    assert_eq!(
        client.get_record_status(&record_hash),
        RecordStatus::Verified
    );
    assert_eq!(client.get_attestation_history(&record_hash).len(), 2);
}

#[test]
fn revoke_attestation_without_revoker_role_fails() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 52);
    attest_with_consent(&env, &client, &attester, &record_hash);

    let outsider = Address::generate(&env);
    assert_eq!(
        client.try_revoke_attestation(&outsider, &record_hash, &Symbol::new(&env, "x")),
        Err(Ok(Error::RoleNotGranted))
    );
    assert!(client.get_attestation(&record_hash).is_some());
}

#[test]
fn revoke_attestation_without_authorization_fails() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 53);
    attest_with_consent(&env, &client, &attester, &record_hash);

    env.mock_auths(&[]);
    assert!(client
        .try_revoke_attestation(&admin, &record_hash, &Symbol::new(&env, "x"))
        .is_err());
}

#[test]
fn revoke_attestation_for_unknown_hash_returns_attestation_not_found() {
    let (env, client, _attester_registry, admin) = setup();
    assert_eq!(
        client.try_revoke_attestation(&admin, &hash(&env, 54), &Symbol::new(&env, "x")),
        Err(Ok(Error::AttestationNotFound))
    );
}

#[test]
fn patient_selected_expiry_marks_attestation_stale() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 55);

    let expires_at = env.ledger().timestamp() + 10;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);
    let attestation = client.attest(&attester, &patient, &record_hash);

    let fresh = client.get_attestation_status(&record_hash).unwrap();
    assert_eq!(fresh.attestation, attestation);
    assert_eq!(fresh.expires_at, Some(expires_at));
    assert!(!fresh.is_expired);

    env.ledger()
        .with_mut(|ledger| ledger.timestamp = expires_at);
    assert!(
        client
            .get_attestation_status(&record_hash)
            .unwrap()
            .is_expired
    );
    assert!(!client.is_verified(&record_hash));
    let history = client.get_attestation_history_status(&record_hash);
    assert_eq!(history.len(), 1);
    assert!(history.get(0).unwrap().is_expired);
}

// ───────────────────────── Admin, roles, pause ─────────────────────────

#[test]
fn propose_admin_by_non_admin_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (new_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_propose_admin(&new_admin).is_err());
}

#[test]
fn accept_admin_by_wrong_address_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);
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
    assert!(client.try_accept_admin().is_err());
}

#[test]
fn accept_admin_with_no_pending_proposal_fails() {
    let (_env, client, _, _admin) = setup();
    assert_eq!(client.try_accept_admin(), Err(Ok(Error::NoPendingTransfer)));
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
    let expected_event = AdminTransferred {
        previous_admin: admin.clone(),
        new_admin: new_admin.clone(),
        contract_kind: Symbol::new(&env, "attestation_registry"),
        schema_version: EVENT_SCHEMA_VERSION,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
    assert_eq!(client.get_admin(), new_admin);

    // The old admin can no longer propose a new admin.
    let newer_admin = Address::generate(&env);
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (newer_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_propose_admin(&newer_admin).is_err());
}

#[test]
fn pause_requires_guardian_and_blocks_attest() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    let record_hash = hash(&env, 60);
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &record_hash, &expires_at);

    let outsider = Address::generate(&env);
    assert_eq!(client.try_pause(&outsider), Err(Ok(Error::RoleNotGranted)));

    client.pause(&admin);
    assert!(client.is_paused());
    assert_eq!(
        client.try_attest(&attester, &patient, &record_hash),
        Err(Ok(Error::ContractPaused))
    );

    client.unpause();
    assert!(!client.is_paused());
    client.attest(&attester, &patient, &record_hash);
}

#[test]
fn roles_can_be_granted_and_revoked_by_admin_only() {
    let (env, client, _attester_registry, _admin) = setup();
    let account = Address::generate(&env);
    assert!(!client.has_role(&Role::Revoker, &account));
    client.grant_role(&Role::Revoker, &account);
    assert!(client.has_role(&Role::Revoker, &account));
    client.revoke_role(&Role::Revoker, &account);
    assert!(!client.has_role(&Role::Revoker, &account));

    env.mock_auths(&[]);
    assert!(client.try_grant_role(&Role::Revoker, &account).is_err());
}

// ───────────────────────── Documentation checks ─────────────────────────

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

// ───────────────────────── Rate limiting ─────────────────────────

fn rate_limited_setup(
    max_per_window: u32,
    window_ledgers: u32,
) -> (Env, AttestationRegistryClient<'static>, Address) {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);
    client.set_attestation_rate_limit(&max_per_window, &window_ledgers);
    (env, client, attester)
}

#[test]
fn rate_limit_disabled_by_default() {
    let (env, client, attester_registry, admin) = setup();
    let attester = enroll(&env, &attester_registry, &admin);

    assert_eq!(client.get_attestation_rate_limit(), None);
    for i in 0..20 {
        attest_with_consent(&env, &client, &attester, &hash(&env, i));
    }
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);
}

#[test]
fn rate_limit_under_and_at_limit_succeeds() {
    let (env, client, attester) = rate_limited_setup(3, 100);

    attest_with_consent(&env, &client, &attester, &hash(&env, 1));
    attest_with_consent(&env, &client, &attester, &hash(&env, 2));
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);

    attest_with_consent(&env, &client, &attester, &hash(&env, 3));
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
    let attestation = attest_with_consent(&env, &client, &attester, &record_hash);

    let hit = RateLimitHit {
        attester: attester.clone(),
        retry_after_ledger: start + 100,
    };
    let recorded = recorded_event(
        &env,
        &record_hash,
        &attestation,
        env.ledger().timestamp() + 10_000,
        1,
        None,
    );
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

    attest_with_consent(&env, &client, &attester, &hash(&env, 1));
    attest_with_consent(&env, &client, &attester, &hash(&env, 2));

    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &hash(&env, 3), &expires_at);
    assert_eq!(
        client.try_attest(&attester, &patient, &hash(&env, 3)),
        Err(Ok(Error::RateLimited))
    );
    assert_eq!(client.get_attestation(&hash(&env, 3)), None);
}

#[test]
fn rate_limit_window_rollover_resets_count() {
    let (env, client, attester) = rate_limited_setup(1, 100);

    attest_with_consent(&env, &client, &attester, &hash(&env, 1));
    advance_ledgers(&env, 99);
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &hash(&env, 2), &expires_at);
    assert_eq!(
        client.try_attest(&attester, &patient, &hash(&env, 2)),
        Err(Ok(Error::RateLimited))
    );

    advance_ledgers(&env, 1);
    assert_eq!(client.get_rate_limit_retry_after(&attester), None);
    client.attest(&attester, &patient, &hash(&env, 2));
}

#[test]
fn rate_limit_is_per_attester() {
    let (env, client, attester_registry, admin) = setup();
    let a = enroll(&env, &attester_registry, &admin);
    let b = enroll(&env, &attester_registry, &admin);
    client.set_attestation_rate_limit(&1, &100);

    attest_with_consent(&env, &client, &a, &hash(&env, 1));
    attest_with_consent(&env, &client, &b, &hash(&env, 2));

    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &a, &hash(&env, 3), &expires_at);
    assert_eq!(
        client.try_attest(&a, &patient, &hash(&env, 3)),
        Err(Ok(Error::RateLimited))
    );
}

#[test]
fn rate_limit_per_attester_override() {
    let (env, client, attester) = rate_limited_setup(1, 100);
    client.set_attester_rate_limit(&attester, &3);

    for i in 0..3 {
        attest_with_consent(&env, &client, &attester, &hash(&env, i));
    }
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &hash(&env, 9), &expires_at);
    assert_eq!(
        client.try_attest(&attester, &patient, &hash(&env, 9)),
        Err(Ok(Error::RateLimited))
    );

    client.remove_attester_rate_limit(&attester);
    advance_ledgers(&env, 100);
    client.attest(&attester, &patient, &hash(&env, 9));
    client.consent_attestation(&patient, &attester, &hash(&env, 11), &(expires_at + 1_000));
    assert_eq!(
        client.try_attest(&attester, &patient, &hash(&env, 11)),
        Err(Ok(Error::RateLimited))
    );
}

#[test]
fn rate_limit_fails_open_after_temporary_entry_expiry() {
    let (env, client, attester) = rate_limited_setup(1, 100);
    attest_with_consent(&env, &client, &attester, &hash(&env, 1));

    // Simulate the temporary `RateWindow` entry being archived before the
    // window ends: the attester starts a fresh window.
    env.as_contract(&client.address, || {
        env.storage()
            .temporary()
            .remove(&DataKey::RateWindow(attester.clone()));
    });

    attest_with_consent(&env, &client, &attester, &hash(&env, 2));
    let patient = Address::generate(&env);
    let expires_at = env.ledger().timestamp() + 10_000;
    client.consent_attestation(&patient, &attester, &hash(&env, 3), &expires_at);
    assert_eq!(
        client.try_attest(&attester, &patient, &hash(&env, 3)),
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
        attest_with_consent(&env, &client, &attester, &hash(&env, i));
    }
}

#[test]
fn set_attestation_rate_limit_requires_admin_auth() {
    let (env, client, _attester_registry, _admin) = setup();
    env.mock_auths(&[]);
    assert!(client.try_set_attestation_rate_limit(&1, &100).is_err());
}
