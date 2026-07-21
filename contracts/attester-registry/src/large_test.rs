//! Load test for large attester allowlist

#[cfg(test)]
mod large_test {
    extern crate std;

    use crate::{AttesterRegistry, AttesterRegistryClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    #[test]
    fn large_attester_allowlist_load() {
        let (env, client, admin) = {
            let env = Env::default();
            env.mock_all_auths();
            let contract_id = env.register(AttesterRegistry, ());
            let client = AttesterRegistryClient::new(&env, &contract_id);
            let admin = Address::generate(&env);
            (env, client, admin)
        };
        client.initialize(&admin);

        let total_attesters: usize = 1000;
        let mut early_attester: Option<Address> = None;
        let mut mid_attester: Option<Address> = None;
        let mut last_attester: Option<Address> = None;

        for i in 0..total_attesters {
            let attester = Address::generate(&env);
            client.add_attester(&attester);
            if i == 0 {
                early_attester = Some(attester.clone());
            } else if i == total_attesters / 2 {
                mid_attester = Some(attester.clone());
            } else if i == total_attesters - 1 {
                last_attester = Some(attester.clone());
            }
        }

        assert!(early_attester.is_some());
        assert!(mid_attester.is_some());
        assert!(last_attester.is_some());
        assert!(client.is_attester(early_attester.as_ref().unwrap()));
        assert!(client.is_attester(mid_attester.as_ref().unwrap()));
        assert!(client.is_attester(last_attester.as_ref().unwrap()));
    }
}
