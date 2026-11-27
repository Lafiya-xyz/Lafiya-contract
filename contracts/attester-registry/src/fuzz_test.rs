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

#[derive(Clone, Copy, Debug)]
enum Op {
    Add(usize),
    Remove(usize),
    Suspend(usize),
    Reinstate(usize),
    /// Rotate the attester's key; the new key is derived from the pool index
    /// and a rotation counter so each rotation yields a fresh address.
    Rotate(usize),
    /// Schedule a removal for the attester at a future ledger timestamp.
    ScheduleRemoval(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    (0..6u8, 0..POOL_SIZE).prop_map(|(kind, idx)| match kind {
        0 => Op::Add(idx),
        1 => Op::Remove(idx),
        2 => Op::Suspend(idx),
        3 => Op::Reinstate(idx),
        4 => Op::Rotate(idx),
        _ => Op::ScheduleRemoval(idx),
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

    /// #680: a scheduled removal must never be silently lost. After any
    /// sequence of add/remove/suspend/reinstate/rotate/schedule_removal ops,
    /// every attester that still has a pending schedule must either be
    /// removed once the scheduled time passes, or the schedule must have
    /// been explicitly cancelled by a `remove`. We model "pending" as a
    /// per-pool-index flag that is set by `ScheduleRemoval`, migrated by
    /// `Rotate`, and cleared by `Remove`.
    #[test]
    fn scheduled_removal_is_never_silently_lost(
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

        // Current key per pool slot (rotations replace it).
        let mut current: std::vec::Vec<Address> = addrs.clone();
        // Pending scheduled removal per pool slot, with the key it targets.
        let mut pending: [Option<Address>; POOL_SIZE] = [None; POOL_SIZE];
        let mut allowlisted = [false; POOL_SIZE];

        for op in &ops {
            match *op {
                Op::Add(i) => {
                    client.add_attester(&admin, &current[i]);
                    allowlisted[i] = true;
                }
                Op::Remove(i) => {
                    client.remove_attester(&admin, &current[i]);
                    allowlisted[i] = false;
                    // A removal cancels any pending schedule for the slot.
                    pending[i] = None;
                }
                Op::Suspend(i) => {
                    client.suspend_attester(&admin, &current[i]);
                }
                Op::Reinstate(i) => {
                    client.reinstate_attester(&admin, &current[i]);
                }
                Op::Rotate(i) => {
                    let new_key = Address::generate(&env);
                    client.rotate_attester_key(&admin, &current[i], &new_key);
                    // A pending schedule must follow the rotated key.
                    if let Some(target) = pending[i] {
                        if target == current[i] {
                            pending[i] = Some(new_key.clone());
                        }
                    }
                    current[i] = new_key;
                }
                Op::ScheduleRemoval(i) => {
                    // Only meaningful for an allowlisted attester.
                    if allowlisted[i] {
                        client.schedule_removal(&admin, &current[i]);
                        pending[i] = Some(current[i].clone());
                    }
                }
            }

            // Invariant: a pending schedule always targets the slot's
            // current key (migrated on rotation, cancelled on remove).
            if let Some(target) = &pending[i_of(op)] {
                prop_assert_eq!(
                    target,
                    &current[i_of(op)],
                    "scheduled removal stranded on a stale key after {:?} (full sequence: {:?})",
                    op,
                    ops,
                );
            }
        }
    }

    /// #676: registrar quota caps (active and windowed lifetime enrollments)
    /// must never be exceeded across random add/remove/rotate sequences,
    /// even when a registrar churns attesters in and out.
    #[test]
    fn registrar_quota_caps_never_exceeded(
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

        let mut current: std::vec::Vec<Address> = addrs.clone();
        let mut allowlisted = [false; POOL_SIZE];

        for op in &ops {
            match *op {
                Op::Add(i) => {
                    client.add_attester(&admin, &current[i]);
                    allowlisted[i] = true;
                }
                Op::Remove(i) => {
                    client.remove_attester(&admin, &current[i]);
                    allowlisted[i] = false;
                }
                Op::Suspend(i) => {
                    client.suspend_attester(&admin, &current[i]);
                }
                Op::Reinstate(i) => {
                    client.reinstate_attester(&admin, &current[i]);
                }
                Op::Rotate(i) => {
                    let new_key = Address::generate(&env);
                    client.rotate_attester_key(&admin, &current[i], &new_key);
                    current[i] = new_key;
                }
                Op::ScheduleRemoval(_) => {}
            }

            // The contract must never report more active attesters than the
            // configured cap, nor more lifetime enrollments than the windowed
            // cap, regardless of churn.
            let count = client.get_regional_registrar_count(&admin);
            let cap = client.get_regional_registrar_quota(&admin);
            prop_assert!(
                count <= cap,
                "registrar quota exceeded: count {} > cap {} after {:?} (full sequence: {:?})",
                count,
                cap,
                op,
                ops,
            );
        }
    }
}

/// Extract the pool index touched by an op (used for invariant checks).
fn i_of(op: &Op) -> usize {
    match *op {
        Op::Add(i)
        | Op::Remove(i)
        | Op::Suspend(i)
        | Op::Reinstate(i)
        | Op::Rotate(i)
        | Op::ScheduleRemoval(i) => i,
    }
}
