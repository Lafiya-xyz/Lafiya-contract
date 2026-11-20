//! Load test for large attester allowlist

extern crate std;

use crate::{AttesterRegistry, AttesterRegistryClient, Role};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

struct BudgetCheckpoint {
    attesters: usize,
    max_cpu_instructions: u64,
    max_memory_bytes: u64,
}

const TOTAL_ATTESTERS: usize = 1_000;

// Native contract tests omit Wasm execution and transaction-envelope costs. These
// ceilings therefore guard relative regressions in add_attester, not network fees.
// Each is deliberately far below the network invocation limit while retaining
// headroom for cost-model adjustments in compatible SDK releases. The 1,000
// attester ceilings include the per-enrollment stale-suspension check, role
// lookup, status-history record and validity bookkeeping.
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
        max_cpu_instructions: 10_000_000,
        max_memory_bytes: 4_000_000,
    },
];

#[test]
fn large_attester_allowlist_load() {
    let (env, client, admin) = {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(AttesterRegistry, (admin.clone(),));
        let client = AttesterRegistryClient::new(&env, &contract_id);
        (env, client, admin)
    };
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);

    let mut sampled_attesters = std::vec::Vec::<Address>::new();
    let mut observed_checkpoints = 0;

    for i in 0..TOTAL_ATTESTERS {
        let attester = Address::generate(&env);
        client.add_attester(&admin, &attester);

        let attester_count = i + 1;
        if let Some(checkpoint) = BUDGET_CHECKPOINTS
            .iter()
            .find(|checkpoint| checkpoint.attesters == attester_count)
        {
            observed_checkpoints += 1;
            let budget = env.cost_estimate().budget();
            let cpu = budget.cpu_instruction_cost();
            let memory = budget.memory_bytes_cost();
            std::println!("add_attester at {attester_count} attesters: cpu={cpu}, memory={memory}");
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

        if i == 0 || i == TOTAL_ATTESTERS / 2 || i == TOTAL_ATTESTERS - 1 {
            sampled_attesters.push(attester);
        }
    }

    assert_eq!(observed_checkpoints, BUDGET_CHECKPOINTS.len());

    assert_eq!(sampled_attesters.len(), 3);
    for attester in &sampled_attesters {
        assert!(client.is_attester(attester));
    }
}

#[test]
fn large_session_invalidation_is_constant_time() {
    let (env, client, admin) = {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(AttesterRegistry, (admin.clone(),));
        let client = AttesterRegistryClient::new(&env, &contract_id);
        (env, client, admin)
    };
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);

    // Fill the attester's session slots up to the cap.
    let mut issued = Vec::new(&env);
    let mut i = 0u32;
    loop {
        match client.try_issue_session_key(&admin, &attester) {
            Ok(Ok(session)) => {
                issued.push_back(session);
                i += 1;
            }
            Ok(Err(err)) => {
                // The cap must be enforced with a typed error.
                assert_eq!(err, crate::Error::SessionLimitReached);
                break;
            }
            Err(_) => panic!("unexpected invocation failure while issuing sessions"),
        }
    }
    assert!(i > 0, "expected at least one session to be issued");
    assert_eq!(i, crate::MAX_SESSIONS_PER_ATTESTER);

    // Every issued session is valid before invalidation.
    for session in issued.iter() {
        assert!(client.is_session_valid(&session));
    }
    assert_eq!(client.list_sessions(&attester).len(), i);

    // Suspending the attester invalidates all sessions with O(1) writes.
    let budget_before = env.cost_estimate().budget();
    let cpu_before = budget_before.cpu_instruction_cost();
    let memory_before = budget_before.memory_bytes_cost();
    client.suspend_attester(&admin, &attester);
    let budget_after = env.cost_estimate().budget();
    let cpu_cost = budget_after.cpu_instruction_cost() - cpu_before;
    let memory_cost = budget_after.memory_bytes_cost() - memory_before;
    std::println!("suspend with {i} sessions: cpu={cpu_cost}, memory={memory_cost}");

    // The invalidation cost must not scale with the number of sessions.
    assert!(
        cpu_cost <= 2_000_000,
        "suspend CPU cost with {i} sessions was {cpu_cost}, exceeding ceiling"
    );
    assert!(
        memory_cost <= 1_000_000,
        "suspend memory cost with {i} sessions was {memory_cost}, exceeding ceiling"
    );

    for session in issued.iter() {
        assert!(!client.is_session_valid(&session));
    }
    assert_eq!(client.list_sessions(&attester).len(), 0);

    // revoke_all_sessions also invalidates in constant time and emits an event.
    client.unsuspend_attester(&admin, &attester);
    let mut reissued = Vec::new(&env);
    for _ in 0..crate::MAX_SESSIONS_PER_ATTESTER {
        reissued.push_back(client.issue_session_key(&admin, &attester));
    }
    for session in reissued.iter() {
        assert!(client.is_session_valid(&session));
    }
    client.revoke_all_sessions(&admin, &attester);
    for session in reissued.iter() {
        assert!(!client.is_session_valid(&session));
    }
    assert_eq!(client.list_sessions(&attester).len(), 0);
}
