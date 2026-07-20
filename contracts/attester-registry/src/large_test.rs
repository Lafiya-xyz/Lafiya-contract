//! Load test for large attester allowlist

#[cfg(test)]
mod large_test {
    use super::*;
    use soroban_sdk::{Env, Address};
    use soroban_sdk::testutils::{Events as _, Address as _};

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
        let mut early_attester: Option<Address> = None;
        let mut mid_attester: Option<Address> = None;
        let mut last_attester: Option<Address> = None;

        for i in 0..total_attesters {
            let attester = Address::generate(&env);
            client.add_attester(&attester);
            // Capture early, middle, and last attesters for verification
            if i == 0 {
                early_attester = Some(attester.clone());
            } else if i == total_attesters / 2 {
                mid_attester = Some(attester.clone());
            } else if i == total_attesters - 1 {
                last_attester = Some(attester.clone());
            }
        }

        // Verify that lookups succeed for selected attesters
        assert!(early_attester.is_some());
        assert!(mid_attester.is_some());
        assert!(last_attester.is_some());
        assert!(client.is_attester(early_attester.as_ref().unwrap()));
        assert!(client.is_attester(mid_attester.as_ref().unwrap()));
        assert!(client.is_attester(last_attester.as_ref().unwrap()));

        // Record resource budget usage (debug output for CI logs)
        let budget = env.budget();
        // These methods exist on Budget; if not, they are placeholders for illustration.
        // In actual Soroban SDK, you can query used CPU instructions and memory.
        // For now we just ensure the test completes without hitting limits.
        println!("Budget after adding {} attesters: {:?}", total_attesters, budget);
    }
}
