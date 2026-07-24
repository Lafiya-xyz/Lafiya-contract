//! Property-based fuzz testing for `attest`. Targets two things the issue
//! called out specifically: panics on arbitrary/adversarial `record_hash`
//! byte patterns, and unusual call orderings relative to `initialize`.
//!
//! Run just this target locally with more cases via:
//! `PROPTEST_CASES=10000 cargo test -p attestation-registry fuzz_test -- --nocapture`

extern crate std;

use super::*;
use attester_registry::{AttesterRegistry, AttesterRegistryClient};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any 32-byte `record_hash` value — including all-zero, all-`0xFF`,
    /// and arbitrary bytes — must be accepted by `attest` for an
    /// allowlisted attester and readable back afterwards, with no panic.
    #[test]
    fn attest_never_panics_on_arbitrary_record_hash(bytes in proptest::array::uniform32(any::<u8>())) {
        let env = Env::default();
        env.mock_all_auths();

        let attester_registry_id = env.register(AttesterRegistry, ());
        let attester_registry_client = AttesterRegistryClient::new(&env, &attester_registry_id);
        let contract_id = env.register(AttestationRegistry, ());
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let attester = Address::generate(&env);
        attester_registry_client.initialize(&admin);
        client.initialize(&admin, &attester_registry_id);
        attester_registry_client.add_attester(&attester);

        let record_hash = BytesN::from_array(&env, &bytes);
        let result = client.try_attest(&attester, &record_hash);
        prop_assert!(result.is_ok());
        prop_assert!(client.get_attestation(&record_hash).is_some());
    }

    /// Calling `attest` before `initialize` must fail cleanly with
    /// `Error::NotInitialized` for any `record_hash`, never panic.
    #[test]
    fn attest_before_initialize_never_panics(bytes in proptest::array::uniform32(any::<u8>())) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(AttestationRegistry, ());
        let client = AttestationRegistryClient::new(&env, &contract_id);
        let attester = Address::generate(&env);
        let record_hash = BytesN::from_array(&env, &bytes);

        let result = client.try_attest(&attester, &record_hash);
        prop_assert_eq!(result, Err(Ok(Error::NotInitialized)));
    }

    /// Re-attesting the same `record_hash` with arbitrary byte content,
    /// any number of times, must always leave `get_attestation` returning
    /// the most recent attester — never panic, never a stale value.
    #[test]
    fn repeated_attest_on_same_hash_never_panics(bytes in proptest::array::uniform32(any::<u8>()), attempts in 1usize..8) {
        let env = Env::default();
        env.mock_all_auths();

        let attester_registry_id = env.register(AttesterRegistry, ());
        let attester_registry_client = AttesterRegistryClient::new(&env, &attester_registry_id);
        let contract_id = env.register(AttestationRegistry, ());
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        attester_registry_client.initialize(&admin);
        client.initialize(&admin, &attester_registry_id);

        let record_hash = BytesN::from_array(&env, &bytes);
        let mut last_attester = None;
        for _ in 0..attempts {
            let attester = Address::generate(&env);
            attester_registry_client.add_attester(&attester);
            let result = client.try_attest(&attester, &record_hash);
            prop_assert!(result.is_ok());
            last_attester = Some(attester);
        }

        if let Some(expected) = last_attester {
            let stored = client.get_attestation(&record_hash);
            prop_assert!(stored.is_some());
            if let Some(attestation) = stored {
                prop_assert_eq!(attestation.attester, expected);
            }
        }
    }
}
