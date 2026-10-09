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
use soroban_sdk::{Address, BytesN, Env};

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
}
