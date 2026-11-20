//! Property-based fuzz testing for the attester-registry allowlist state
//! machine. Generates arbitrary sequences of add/remove/suspend/reinstate
//! calls over a small pool of addresses and checks, after *every* step,
//! that `is_attester` agrees with a plain-Rust model of "allowlisted and
//! not suspended" — i.e. it never observes an address as both allowlisted
//! and not (or vice versa) for a single input value.
//!
//! Run just this target locally with more cases via:
//! `PROPTEST_CASES=10000 cargo test -p attester-registry fuzz_test -- --nocapture`

extern crate std;

use super::*;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const POOL_SIZE: usize = 4;

/// Number of cases for the bond accounting stateful test. The default is
/// kept modest so the suite stays fast on PRs; nightly runs set
/// `PROPTEST_CASES=10000` (or higher) to exercise the full invariant.
fn bond_cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(256)
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
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);

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
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();

        for i in ops {
            client.add_attester(&admin, &addrs[i]);
            prop_assert!(client.is_attester(&addrs[i]));
        }
    }
}

// ---------------------------------------------------------------------------
// Bond / slashing accounting invariant
// ---------------------------------------------------------------------------
//
// Invariant under test (checked after *every* step):
//
//     sum(bonds) + slashed_unswept == token.balance(contract)
//
// Slashed funds are held in `slashed_unswept` until the treasury sweeps
// them; only the treasury role may sweep. A fee-charging token is modelled
// by having the mock token deduct a fee on each transfer, so the contract
// must reconcile against actual balance deltas rather than nominal amounts.

#[derive(Clone, Copy, Debug)]
enum BondOp {
    /// Deposit `amount` of bond for pool index `idx`.
    Bond(usize, i128),
    /// Withdraw the full bond for pool index `idx`.
    Unbond(usize),
    /// Slash `amount` from pool index `idx`'s bond.
    Slash(usize, i128),
    /// Treasury sweeps all unswept slashed funds.
    Sweep,
}

fn bond_op_strategy() -> impl Strategy<Value = BondOp> {
    (0..4u8, 0..POOL_SIZE, 1..1_000i128).prop_map(|(kind, idx, amount)| match kind {
        0 => BondOp::Bond(idx, amount),
        1 => BondOp::Unbond(idx),
        2 => BondOp::Slash(idx, amount),
        _ => BondOp::Sweep,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(bond_cases()))]

    #[test]
    fn bond_accounting_invariant_holds_after_any_op_sequence(
        ops in proptest::collection::vec(bond_op_strategy(), 0..64)
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.grant_role(&Role::Registrar, &admin);
        client.grant_role(&Role::Guardian, &admin);
        client.grant_role(&Role::Treasury, &admin);

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();

        // Plain-Rust model of per-attester bonds and unswept slashed funds.
        let mut bonds = [0i128; POOL_SIZE];
        let mut slashed_unswept: i128 = 0;

        for op in &ops {
            match *op {
                BondOp::Bond(i, amount) => {
                    client.bond(&addrs[i], &amount);
                    bonds[i] += amount;
                }
                BondOp::Unbond(i) => {
                    let before = bonds[i];
                    client.unbond(&addrs[i]);
                    bonds[i] = 0;
                    let _ = before;
                }
                BondOp::Slash(i, amount) => {
                    let slashable = bonds[i].min(amount);
                    client.slash(&addrs[i], &slashable);
                    bonds[i] -= slashable;
                    slashed_unswept += slashable;
                }
                BondOp::Sweep => {
                    client.sweep_slashed(&admin);
                    slashed_unswept = 0;
                }
            }

            // Contract-side view must agree with the model.
            let mut model_sum: i128 = 0;
            for i in 0..POOL_SIZE {
                prop_assert_eq!(
                    client.bond_of(&addrs[i]),
                    bonds[i],
                    "bond_of mismatch for pool index {} after {:?} (full sequence: {:?})",
                    i,
                    op,
                    ops,
                );
                model_sum += bonds[i];
            }
            prop_assert_eq!(
                client.slashed_unswept(),
                slashed_unswept,
                "slashed_unswept mismatch after {:?} (full sequence: {:?})",
                op,
                ops,
            );

            // The core accounting invariant, reconciled against the actual
            // token balance held by the contract.
            let contract_balance = client.token_balance();
            prop_assert_eq!(
                model_sum + slashed_unswept,
                contract_balance,
                "bond invariant violated after {:?} (full sequence: {:?})",
                op,
                ops,
            );

            // The on-chain reconcile view must never report drift.
            prop_assert!(
                !client.bond_reconcile(),
                "bond_reconcile reported drift after {:?} (full sequence: {:?})",
                op,
                ops,
            );
        }
    }
}
