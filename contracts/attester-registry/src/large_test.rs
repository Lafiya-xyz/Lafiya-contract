//! Load test for large attester allowlist

#[cfg(test)]
mod large_test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    struct BudgetCheckpoint {
        attesters: usize,
        max_cpu_instructions: u64,
        max_memory_bytes: u64,
    }

    const TOTAL_ATTESTERS: usize = 1_000;

    // Native contract tests omit Wasm execution and transaction-envelope costs. These
    // ceilings therefore guard relative regressions in add_attester, not network fees.
    // Each is deliberately far below the network invocation limit while retaining
    // headroom for cost-model adjustments in compatible SDK releases.
    const BUDGET_CHECKPOINTS: [BudgetCheckpoint; 3] = [
        BudgetCheckpoint {
            attesters: 10,
            max_cpu_instructions: 2_000_000,
            max_memory_bytes: 1_000_000,
        },
        BudgetCheckpoint {
            attesters: 100,
            max_cpu_instructions: 2_000_000,
            max_memory_bytes: 1_000_000,
        },
        BudgetCheckpoint {
            attesters: 1_000,
            max_cpu_instructions: 2_000_000,
            max_memory_bytes: 1_000_000,
        },
    ];

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

        let mut sampled_attesters = std::vec::Vec::<Address>::new();
        let mut observed_checkpoints = 0;

        for i in 0..TOTAL_ATTESTERS {
            let attester = Address::generate(&env);
            client.add_attester(&attester);

            let attester_count = i + 1;
            if let Some(checkpoint) = BUDGET_CHECKPOINTS
                .iter()
                .find(|checkpoint| checkpoint.attesters == attester_count)
            {
                observed_checkpoints += 1;
                // Soroban resets budget metering before each top-level invocation, so
                // this measures the add that ran with the stated allowlist size.
                let budget = env.budget();
                let cpu = budget.cpu_instruction_cost();
                let memory = budget.memory_bytes_cost();
                println!(
                    "add_attester at {attester_count} attesters: cpu={cpu}, memory={memory}"
                );
                assert!(
                    cpu <= checkpoint.max_cpu_instructions,
                    "add_attester CPU cost at {} attesters was {}, exceeding ceiling {}",
                    checkpoint.attesters,
                    cpu,
                    checkpoint.max_cpu_instructions
                );
                assert!(
                    memory <= checkpoint.max_memory_bytes,
                    "add_attester memory cost at {} attesters was {}, exceeding ceiling {}",
                    checkpoint.attesters,
                    memory,
                    checkpoint.max_memory_bytes
                );
            }

            // Capture early, middle, and last attesters for verification
            if i == 0 || i == TOTAL_ATTESTERS / 2 || i == TOTAL_ATTESTERS - 1 {
                sampled_attesters.push(attester);
            }
        }

        assert_eq!(observed_checkpoints, BUDGET_CHECKPOINTS.len());

        // Verify that lookups succeed for selected attesters
        assert_eq!(sampled_attesters.len(), 3);
        for attester in &sampled_attesters {
            assert!(client.is_attester(attester));
        }
    }
}
