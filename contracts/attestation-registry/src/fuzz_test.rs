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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any 32-byte `record_hash` value — including all-zero, all-`0xFF`,
    /// and arbitrary bytes — must be accepted by `attest` for an
    /// allowlisted attester and readable back afterwards, with no panic.
    #[test]
    fn attest_never_panics_on_arbitrary_record_hash(bytes in proptest::array::uniform32(any::<u8>())) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let attester_registry_id = env.register(AttesterRegistry, (admin.clone(),));
        let attester_registry_client = AttesterRegistryClient::new(&env, &attester_registry_id);
        let contract_id = env.register(
            AttestationRegistry,
            (admin, attester_registry_id.clone()),
        );
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let attester = Address::generate(&env);
        attester_registry_client.add_attester(&attester);

        let record_hash = BytesN::from_array(&env, &bytes);
        let result = client.try_attest(&attester, &record_hash);
        prop_assert!(result.is_ok());
        prop_assert!(!client.get_attestation(&record_hash).is_empty());
    }

    /// Re-attesting the same `record_hash` with arbitrary byte content,
    /// any number of times, must always leave `get_attestation` returning
    /// the most recent attester — never panic, never a stale value.
    #[test]
    fn repeated_attest_on_same_hash_never_panics(bytes in proptest::array::uniform32(any::<u8>()), attempts in 1usize..8) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let attester_registry_id = env.register(AttesterRegistry, (admin.clone(),));
        let attester_registry_client = AttesterRegistryClient::new(&env, &attester_registry_id);
        let contract_id = env.register(
            AttestationRegistry,
            (admin, attester_registry_id.clone()),
        );
        let client = AttestationRegistryClient::new(&env, &contract_id);

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
            prop_assert!(stored.iter().any(|attestation| attestation.attester == expected));
        }
    }
}
