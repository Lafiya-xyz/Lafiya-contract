extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{BytesN, Env, Event};

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
    client.initialize(&admin, &attester_registry_id);

    (env, client, attester_registry_client, admin)
}

#[test]
fn initialize_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin, &attester_registry_id);

    let expected_event = Initialized {
        admin: admin.clone(),
        attester_registry: attester_registry_id.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn attest_by_allowlisted_attester_succeeds() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[7u8; 32]);
    let attestation = client.attest(&attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn attest_by_non_allowlisted_attester_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[1u8; 32]);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[2u8; 32]);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn get_attestation_returns_none_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[9u8; 32]);
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn re_attest_overwrites_previous_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);

    let record_hash = BytesN::from_array(&env, &[3u8; 32]);
    client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), Some(second));
}

#[test]
fn attest_emits_event() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[4u8; 32]);

    let attestation = client.attest(&attester, &record_hash);

    let expected_event = AttestationRecorded {
        record_hash: record_hash.clone(),
        attester: attestation.attester.clone(),
        timestamp: attestation.timestamp,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn attest_without_attester_auth_fails() {
    let (env, client, attester_registry, admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[5u8; 32]);
    let _ = &admin;

    // Replace the blanket auth mock with an empty set: the attest call's
    // `attester.require_auth()` now has no matching auth entry to satisfy it.
    env.mock_auths(&[]);
    let result = client.try_attest(&attester, &record_hash);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}
