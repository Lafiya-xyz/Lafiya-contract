//! Load test for large attester allowlist

extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn large_attester_allowlist_load() {
    // Setup environment and contract client
    let (env, client, admin) = {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(AttesterRegistry, ());
        let client = AttesterRegistryClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        (env, client, admin)
    };
    // Initialize with admin
    client.initialize(&admin);

    // Define number of attesters to add
    let total_attesters: usize = 1000; // Adjust if resource limits encountered
    let mut attesters: std::vec::Vec<Address> = std::vec::Vec::with_capacity(total_attesters);

    for _ in 0..total_attesters {
        let attester = Address::generate(&env);
        client.add_attester(&attester);
        attesters.push(attester);
    }

    // Verify that lookups succeed for the first, middle, and last attesters added.
    let early_attester = &attesters[0];
    let mid_attester = &attesters[total_attesters / 2];
    let last_attester = &attesters[total_attesters - 1];
    assert!(client.is_attester(early_attester));
    assert!(client.is_attester(mid_attester));
    assert!(client.is_attester(last_attester));

    // Record resource budget usage (debug output for CI logs)
    let budget = env.cost_estimate().budget();
    std::println!(
        "Budget after adding {} attesters: {:?}",
        total_attesters,
        budget
    );
}
