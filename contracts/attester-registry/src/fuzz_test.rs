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
}

fn op_strategy() -> impl Strategy<Value = Op> {
    (0..4u8, 0..POOL_SIZE).prop_map(|(kind, idx)| match kind {
        0 => Op::Add(idx),
        1 => Op::Remove(idx),
        2 => Op::Suspend(idx),
        _ => Op::Reinstate(idx),
    })
}

/// Operations that exercise the session-generation machinery: issuing
/// sessions, revoking all of an attester's sessions, and the lifecycle
/// transitions (suspend / reinstate / remove / rotate) that must bump the
/// generation and thereby invalidate every outstanding session in O(1).
#[derive(Clone, Copy, Debug)]
enum SessionOp {
    Add(usize),
    Issue(usize),
    RevokeAll(usize),
    Suspend(usize),
    Reinstate(usize),
    Remove(usize),
    Rotate(usize),
}

fn session_op_strategy() -> impl Strategy<Value = SessionOp> {
    (0..7u8, 0..POOL_SIZE).prop_map(|(kind, idx)| match kind {
        0 => SessionOp::Add(idx),
        1 => SessionOp::Issue(idx),
        2 => SessionOp::RevokeAll(idx),
        3 => SessionOp::Suspend(idx),
        4 => SessionOp::Reinstate(idx),
        5 => SessionOp::Remove(idx),
        _ => SessionOp::Rotate(idx),
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

    #[test]
    fn session_generation_invalidates_sessions_after_any_op_sequence(
        ops in proptest::collection::vec(session_op_strategy(), 0..48)
    ) {
        // Model: for each pool index we track whether the attester is
        // currently allowlisted and the generation under which the most
        // recently issued session was created. A session is valid iff the
        // attester is allowlisted and the recorded generation still equals
        // the contract's current generation for that attester.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.grant_role(&Role::Registrar, &admin);
        client.grant_role(&Role::Guardian, &admin);

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();

        let mut allowlisted = [false; POOL_SIZE];
        // Generation the model believes is current for each attester.
        let mut generation = [0u32; POOL_SIZE];
        // Generation recorded on the last issued session (if any).
        let mut issued: [Option<u32>; POOL_SIZE] = [None; POOL_SIZE];

        for op in &ops {
            match *op {
                SessionOp::Add(i) => {
                    client.add_attester(&admin, &addrs[i]);
                    allowlisted[i] = true;
                }
                SessionOp::Issue(i) => {
                    // Issuing may fail (e.g. attester not allowlisted or the
                    // per-attester session cap is hit); only record the
                    // session when the contract accepted it.
                    let key = client.try_issue_session(&admin, &addrs[i]);
                    if let Ok(Ok(session)) = key {
                        issued[i] = Some(session.generation);
                    }
                }
                SessionOp::RevokeAll(i) => {
                    let _ = client.try_revoke_all_sessions(&admin, &addrs[i]);
                    generation[i] = generation[i].wrapping_add(1);
                    issued[i] = None;
                }
                SessionOp::Suspend(i) => {
                    let _ = client.try_suspend_attester(&admin, &addrs[i]);
                    generation[i] = generation[i].wrapping_add(1);
                    issued[i] = None;
                }
                SessionOp::Reinstate(i) => {
                    let _ = client.try_reinstate_attester(&admin, &addrs[i]);
                    generation[i] = generation[i].wrapping_add(1);
                    issued[i] = None;
                }
                SessionOp::Remove(i) => {
                    let _ = client.try_remove_attester(&admin, &addrs[i]);
                    allowlisted[i] = false;
                    generation[i] = generation[i].wrapping_add(1);
                    issued[i] = None;
                }
                SessionOp::Rotate(i) => {
                    let _ = client.try_rotate_attester(&admin, &addrs[i]);
                    generation[i] = generation[i].wrapping_add(1);
                    issued[i] = None;
                }
            }

            for i in 0..POOL_SIZE {
                // Any session recorded before an invalidating op must now be
                // rejected; the model clears `issued` on every bump, so a
                // stale session can only survive if the contract failed to
                // bump the generation.
                if let Some(gen) = issued[i] {
                    prop_assert_eq!(
                        gen,
                        generation[i],
                        "session generation drifted for pool index {} after {:?} (full sequence: {:?})",
                        i,
                        op,
                        ops,
                    );
                }
                // The contract's view of the current generation must agree
                // with the model after every step.
                prop_assert_eq!(
                    client.session_generation(&addrs[i]),
                    generation[i],
                    "session_generation mismatch for pool index {} after {:?} (full sequence: {:?})",
                    i,
                    op,
                    ops,
                );
            }
        }
    }

    #[test]
    fn session_cap_is_enforced(
        count in 1..(MAX_SESSIONS_PER_ATTESTER as usize + 8)
    ) {
        // Issuing more than MAX_SESSIONS_PER_ATTESTER sessions for a single
        // attester must fail with the typed cap error rather than growing
        // unbounded.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.grant_role(&Role::Registrar, &admin);
        client.grant_role(&Role::Guardian, &admin);

        let attester = Address::generate(&env);
        client.add_attester(&admin, &attester);

        let mut accepted = 0usize;
        for _ in 0..count {
            match client.try_issue_session(&admin, &attester) {
                Ok(Ok(_)) => accepted += 1,
                Ok(Err(e)) => {
                    prop_assert_eq!(
                        e,
                        Error::SessionLimitReached,
                        "unexpected error while issuing session",
                    );
                    prop_assert_eq!(accepted, MAX_SESSIONS_PER_ATTESTER as usize);
                }
                Err(_) => prop_assert!(false, "issue_session unexpectedly trapped"),
            }
        }
        prop_assert!(accepted <= MAX_SESSIONS_PER_ATTESTER as usize);
        prop_assert_eq!(
            client.list_sessions(&attester).len() as usize,
            accepted,
            "list_sessions length must match accepted sessions",
        );
    }

    #[test]
    fn revoke_all_sessions_clears_list_and_bumps_generation(
        ops in proptest::collection::vec(0..POOL_SIZE, 0..16)
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.grant_role(&Role::Registrar, &admin);
        client.grant_role(&Role::Guardian, &admin);

        let addrs: std::vec::Vec<Address> =
            (0..POOL_SIZE).map(|_| Address::generate(&env)).collect();
        for a in &addrs {
            client.add_attester(&admin, a);
        }

        for i in ops {
            let before = client.session_generation(&addrs[i]);
            let _ = client.try_issue_session(&admin, &addrs[i]);
            client.revoke_all_sessions(&admin, &addrs[i]);
            prop_assert_eq!(
                client.session_generation(&addrs[i]),
                before.wrapping_add(1),
                "revoke_all_sessions must bump the generation exactly once",
            );
            prop_assert_eq!(
                client.list_sessions(&addrs[i]).len(),
                0,
                "revoke_all_sessions must leave no listed sessions",
            );
        }
    }
}
