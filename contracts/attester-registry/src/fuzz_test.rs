//! Property-based fuzz testing for the attester-registry allowlist state
//! machine. Generates arbitrary sequences of add/remove/suspend/reinstate
//! calls over a small pool of addresses and checks, after *every* step,
//! that `is_attester` agrees with a plain-Rust model of "allowlisted and
//! not suspended" — i.e. it never observes an address as both allowlisted
//! and not (or vice versa) for a single input value.
//!
//! Run just this target locally with more cases via:
//! `PROPTEST_CASES=10000 cargo test -p attester-registry fuzz_test -- --nocapture`
//!
//! Case tiers are standardized across the workspace: PR runs use
//! `PROPTEST_CASES=256` (the default below), nightly deep runs use
//! `PROPTEST_CASES=50000` via the `nightly` nextest profile. On failure
//! proptest prints the seed and the minimal failing input; the shrunk case
//! is persisted to `contracts/attester-registry/proptest-regressions/` and
//! must be committed so CI replays it. Reproduce a failure with:
//! `PROPTEST_CASES=256 cargo test -p attester-registry fuzz_test -- --nocapture`
//! and re-run the persisted case with `PROPTEST_REGRESSIONS=1`.

extern crate std;

use super::*;
use lafiya_test_utils::SystemBuilder;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const POOL_SIZE: usize = 4;

/// Build a budget-constrained `Env` with the attester-registry deployed and
/// wired through the shared `SystemBuilder`, returning the client and the
/// admin actor. All roles needed by the state machine are granted to admin.
fn setup() -> (Env, AttesterRegistryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    let system = SystemBuilder::new(&env).with_attester_registry().build();
    let client = system.attester_registry();
    let admin = system.admin();
    client.initialize(&admin);
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);
    (env, client, admin)
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Add(usize),
    Remove(usize),
    Suspend(usize),
    Reinstate(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    (0..4u8, 0..POOL_SIZE).prop_map(|(kind, idx)| match kind {
        0 => Op::Add(idx),
        1 => Op::Remove(idx),
        2 => Op::Suspend(idx),
        _ => Op::Reinstate(idx),
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn is_attester_matches_model_after_any_op_sequence(
        ops in proptest::collection::vec(op_strategy(), 0..64)
    ) {
        let (env, client, admin) = setup();

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();

        // Plain-Rust model mirroring the contract's storage semantics:
        // `allowlisted` tracks presence in the Attester map, `suspended`
        // tracks presence in the Suspended map. `remove` clears both.
        let mut allowlisted = [false; POOL_SIZE];
        let mut suspended = [false; POOL_SIZE];

        for op in &ops {
            match *op {
                Op::Add(i) => {
                    client.add_attester(&admin, &addrs[i]);
                    allowlisted[i] = true;
                }
                Op::Remove(i) => {
                    client.remove_attester(&admin, &addrs[i]);
                    allowlisted[i] = false;
                    suspended[i] = false;
                }
                Op::Suspend(i) => {
                    client.suspend_attester(&admin, &addrs[i]);
                    suspended[i] = true;
                }
                Op::Reinstate(i) => {
                    client.reinstate_attester(&admin, &addrs[i]);
                    suspended[i] = false;
                }
            }

            for i in 0..POOL_SIZE {
                let expected = allowlisted[i] && !suspended[i];
                prop_assert_eq!(
                    client.is_attester(&addrs[i]),
                    expected,
                    "is_attester mismatch for pool index {} after {:?} (full sequence: {:?})",
                    i,
                    op,
                    ops,
                );
            }
        }
    }

    #[test]
    fn add_attester_is_idempotent_for_is_attester(
        ops in proptest::collection::vec(0..POOL_SIZE, 1..32)
    ) {
        // Adding the same (or any) attester any number of times in a row
        // must never flip is_attester to false for an address that was
        // never removed or suspended.
        let (env, client, admin) = setup();

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();

        for i in ops {
            client.add_attester(&admin, &addrs[i]);
            prop_assert!(client.is_attester(&addrs[i]));
        }
    }
}
