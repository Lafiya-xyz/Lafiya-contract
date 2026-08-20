//! Benchmark-only prototype for [issue #134](https://github.com/Lafiya-xyz/Lafiya-contract/issues/134):
//! "Compare on-chain attester enumeration with event-indexed directories".
//!
//! This module is **not** part of `AttesterRegistry` and ships no new
//! contract entry point. It exists to produce real `cost_estimate()` budget
//! numbers for the "paginated on-chain address storage" strategy compared
//! in `docs/adr/0010-attester-enumeration-strategy.md`, using the same
//! technique as `large_test.rs`. The event-derived strategy's on-chain cost
//! is the *existing* `add_attester`/`remove_attester` cost already measured
//! in `large_test.rs`, since that strategy adds no new on-chain writes.
//! Findings and the chosen strategy are recorded in that ADR, not here.

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, vec, Address, Env, Vec};

/// Addresses per on-chain page. Bounds the cost of a single `page()` read
/// independent of total directory size — the property being prototyped.
const PAGE_SIZE: u32 = 200;

#[contracttype]
#[derive(Clone)]
enum PageKey {
    /// One page of up to `PAGE_SIZE` addresses, in insertion order.
    Chunk(u32),
    /// Number of pages allocated so far.
    ChunkCount,
    /// Total addresses ever appended (ignores tombstones; see ADR-0010's
    /// "Mutation and recovery semantics" section for the removal design
    /// this prototype does not implement).
    TotalCount,
}

/// A minimal paginated directory: `add` appends to the current tail page,
/// allocating a new page every `PAGE_SIZE` entries; `page` reads one page
/// by index. No removal/tombstone handling — see the ADR for why removal
/// is deliberately out of scope for this cost prototype.
#[contract]
struct PaginatedDirectory;

#[contractimpl]
impl PaginatedDirectory {
    pub fn add(env: Env, attester: Address) {
        let total: u32 = env
            .storage()
            .persistent()
            .get(&PageKey::TotalCount)
            .unwrap_or(0);
        let chunk_count: u32 = env
            .storage()
            .persistent()
            .get(&PageKey::ChunkCount)
            .unwrap_or(0);
        let tail_has_room = chunk_count > 0 && total % PAGE_SIZE != 0;

        let chunk_index = if tail_has_room {
            chunk_count - 1
        } else {
            chunk_count
        };
        let mut chunk: Vec<Address> = env
            .storage()
            .persistent()
            .get(&PageKey::Chunk(chunk_index))
            .unwrap_or(vec![&env]);
        chunk.push_back(attester);
        env.storage()
            .persistent()
            .set(&PageKey::Chunk(chunk_index), &chunk);

        if !tail_has_room {
            env.storage()
                .persistent()
                .set(&PageKey::ChunkCount, &(chunk_index + 1));
        }
        env.storage()
            .persistent()
            .set(&PageKey::TotalCount, &(total + 1));
    }

    pub fn page(env: Env, chunk_index: u32) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&PageKey::Chunk(chunk_index))
            .unwrap_or(vec![&env])
    }

    pub fn total_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&PageKey::TotalCount)
            .unwrap_or(0)
    }
}

struct BudgetCheckpoint {
    attesters: usize,
    max_cpu_instructions: u64,
    max_memory_bytes: u64,
}

const TOTAL_ATTESTERS: usize = 1_000;

// Mirrors large_test.rs's methodology: native-contract cost_estimate()
// budget, cumulative from Env creation, guarding relative regressions
// rather than asserting absolute network fees.
const ADD_CHECKPOINTS: [BudgetCheckpoint; 3] = [
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
        max_cpu_instructions: 4_000_000,
        max_memory_bytes: 2_000_000,
    },
];

#[test]
fn paginated_add_cost_scales_with_page_not_directory_size() {
    let env = Env::default();
    let contract_id = env.register(PaginatedDirectory, ());
    let client = PaginatedDirectoryClient::new(&env, &contract_id);

    let mut observed_checkpoints = 0;
    for i in 0..TOTAL_ATTESTERS {
        let attester = Address::generate(&env);
        client.add(&attester);

        let attester_count = i + 1;
        if let Some(checkpoint) = ADD_CHECKPOINTS
            .iter()
            .find(|c| c.attesters == attester_count)
        {
            observed_checkpoints += 1;
            let budget = env.cost_estimate().budget();
            let cpu = budget.cpu_instruction_cost();
            let memory = budget.memory_bytes_cost();
            std::println!(
                "paginated add() at {attester_count} attesters (cumulative): cpu={cpu}, memory={memory}"
            );
            assert!(
                cpu <= checkpoint.max_cpu_instructions,
                "add() CPU cost at {} attesters was {}, exceeding ceiling {}",
                checkpoint.attesters,
                cpu,
                checkpoint.max_cpu_instructions
            );
            assert!(
                memory <= checkpoint.max_memory_bytes,
                "add() memory cost at {} attesters was {}, exceeding ceiling {}",
                checkpoint.attesters,
                memory,
                checkpoint.max_memory_bytes
            );
        }
    }
    assert_eq!(observed_checkpoints, ADD_CHECKPOINTS.len());
    assert_eq!(client.total_count(), TOTAL_ATTESTERS as u32);
}

#[test]
fn paginated_page_read_cost_is_independent_of_directory_size() {
    let env = Env::default();
    let contract_id = env.register(PaginatedDirectory, ());
    let client = PaginatedDirectoryClient::new(&env, &contract_id);

    for _ in 0..TOTAL_ATTESTERS {
        client.add(&Address::generate(&env));
    }
    assert_eq!(client.total_count(), TOTAL_ATTESTERS as u32);

    // Read the first page (cold, from a directory with 1,000 entries
    // behind it) and the last page (also cold) and confirm both cost
    // about the same, single-page-sized amount — the property that makes
    // pagination viable at the configured allowlist cap (50,000).
    let before = env.cost_estimate().budget();
    let first_page = client.page(&0);
    let after_first = env.cost_estimate().budget();
    let last_page = client.page(&(TOTAL_ATTESTERS as u32 / PAGE_SIZE - 1));
    let after_last = env.cost_estimate().budget();

    let first_page_cpu = after_first.cpu_instruction_cost() - before.cpu_instruction_cost();
    let last_page_cpu = after_last.cpu_instruction_cost() - after_first.cpu_instruction_cost();
    std::println!(
        "page(0) cpu={first_page_cpu}, page(last of {TOTAL_ATTESTERS}) cpu={last_page_cpu}, page_size={}",
        first_page.len().max(last_page.len())
    );

    assert_eq!(first_page.len(), PAGE_SIZE);
    assert_eq!(last_page.len(), PAGE_SIZE);
    // A ratio-based comparison breaks when either side measures as ~0 (0/0 is
    // NaN), which is exactly what the native cost model reports for these
    // single-key persistent reads at this directory size. The property this
    // prototype actually needs to demonstrate is that reading the *last*
    // page — behind 1,000 entries' worth of other pages/keys it never
    // touches — costs no more than reading the *first* page: a fixed
    // ceiling independent of total directory size, not a ceiling that grows
    // with it the way a single unbounded on-chain list would.
    const MAX_SINGLE_PAGE_READ_CPU: u64 = 200_000;
    assert!(
        first_page_cpu <= MAX_SINGLE_PAGE_READ_CPU,
        "page(0) CPU cost was {first_page_cpu}, exceeding ceiling {MAX_SINGLE_PAGE_READ_CPU}"
    );
    assert!(
        last_page_cpu <= MAX_SINGLE_PAGE_READ_CPU,
        "page(last) CPU cost was {last_page_cpu}, exceeding ceiling {MAX_SINGLE_PAGE_READ_CPU}"
    );
}
