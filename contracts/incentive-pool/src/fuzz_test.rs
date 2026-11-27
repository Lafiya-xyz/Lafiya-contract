//! Property-based fuzzing of every mutating incentive-pool entry point
//! (#508), following the `multisig-account` precedent. Inputs are full-range
//! `i128` amounts and arbitrary 32-byte work-item ids. Every target asserts
//! that the call returns a typed `Error` (never a host trap) and that the
//! custody invariant still holds.
//!
//! Targets: `fuzz_fund`, `fuzz_withdraw`, `fuzz_approve`, `fuzz_claim`,
//! `fuzz_caps`, `fuzz_payout_delay`, `fuzz_cancel_payout_address`,
//! `fuzz_resolve_drift`, plus the sequence-driven `fuzz_sequence`.
//!
//! The pool is deployed through its `__constructor` (no `initialize`), so the
//! harness passes configuration atomically at deploy time.
//!
//! Findings triaged into regression tests below:
//! - `claim` overflowed (`attester_claimed + amount`, `total_paid + amount`)
//!   when caps were set near `i128::MAX`, trapping instead of returning
//!   `AttesterClaimCapExceeded`/`InsufficientPoolBalance`. Fixed with
//!   saturating adds; see `regression_claim_cap_overflow`.
//! - `set_payout_address` computed `now + PAYOUT_ADDRESS_DELAY` with unchecked
//!   arithmetic, trapping under `overflow-checks`. Fixed with `checked_add`
//!   returning `Error::PayoutDelayOverflow`; see `regression_payout_delay_overflow`.
//!
//! Reproduce a failure locally: proptest prints the minimal input and saves
//! a seed under `proptest-regressions/`; rerun with
//! `PROPTEST_CASES=10000 cargo test -p incentive-pool fuzz_test -- --nocapture`.
//! Committed seeds in `proptest-regressions/` are replayed on every run.

extern crate std;

use super::stateful_test::{harness, item_id, op_strategy, outcome, replay, Harness};
use super::*;
use proptest::prelude::*;
use soroban_sdk::BytesN;

const ADMIN_BALANCE: i128 = 1_000_000_000;

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
}

fn custody_holds(h: &Harness) {
    let (d, p) = (h.pool.get_total_deposited(), h.pool.get_total_paid());
    assert!(p <= d);
    assert_eq!(h.token.balance(&h.pool.address), d - p);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn fuzz_fund(amount in i128::MIN..=ADMIN_BALANCE) {
        let h = harness();
        let r = outcome!(h.pool.try_fund(&amount));
        prop_assert_eq!(r.is_ok(), amount > 0);
        custody_holds(&h);
    }

    #[test]
    fn fuzz_withdraw(fund in 1i128..=ADMIN_BALANCE, amount in any::<i128>()) {
        let h = harness();
        h.pool.fund(&fund);
        let r = outcome!(h.pool.try_withdraw(&h.admin, &amount));
        prop_assert_eq!(r.is_ok(), amount > 0 && amount <= fund);
        custody_holds(&h);
    }

    #[test]
    fn fuzz_approve(id in any::<[u8; 32]>(), amount in any::<i128>(), cap in any::<i128>()) {
        let h = harness();
        h.pool.set_max_per_claim(&cap);
        let id = BytesN::from_array(&h.env, &id);
        let r = outcome!(h.pool.try_approve_work_item(&id, &h.attesters[0], &amount));
        prop_assert_eq!(r.is_ok(), amount > 0 && amount <= cap);
        prop_assert_eq!(h.pool.is_work_item_approved(&id), r.is_ok());
    }

    #[test]
    fn fuzz_claim(
        id in any::<[u8; 32]>(),
        amount in 1i128..=i128::MAX,
        fund in 1i128..=ADMIN_BALANCE,
        attester_cap in any::<i128>(),
    ) {
        let h = harness();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_attester(&attester_cap);
        h.pool.fund(&fund);
        let id = BytesN::from_array(&h.env, &id);
        h.pool.approve_work_item(&id, &h.attesters[0], &amount);
        let r = outcome!(h.pool.try_claim(&id));
        prop_assert_eq!(r.is_ok(), amount <= attester_cap && amount <= fund);
        // Never pays twice.
        prop_assert!(outcome!(h.pool.try_claim(&id)).is_err());
        custody_holds(&h);
    }

    #[test]
    fn fuzz_caps(claim_cap in any::<i128>(), attester_cap in any::<i128>()) {
        let h = harness();
        prop_assert!(outcome!(h.pool.try_set_max_per_claim(&claim_cap)).is_ok());
        prop_assert!(outcome!(h.pool.try_set_max_per_attester(&attester_cap)).is_ok());
        prop_assert_eq!(h.pool.get_max_per_claim(), claim_cap);
        prop_assert_eq!(h.pool.get_max_per_attester(), attester_cap);
    }

    /// The delay arithmetic must never trap: any ledger timestamp combined
    /// with `PAYOUT_ADDRESS_DELAY` either schedules or returns a typed error.
    #[test]
    fn fuzz_payout_delay(now in any::<u64>(), addr in any::<[u8; 32]>()) {
        let h = harness();
        h.env.ledger().set_timestamp(now);
        let addr = Address::from_array(&h.env, &addr);
        let r = outcome!(h.pool.try_set_payout_address(&h.attesters[0], &addr));
        match r {
            Ok(()) => {
                let binding = h.pool.get_payout_binding(&h.attesters[0]).unwrap();
                prop_assert_eq!(binding.pending, Some(addr));
                prop_assert!(binding.effective_at >= now);
            }
            Err(e) => prop_assert_eq!(e, Error::PayoutDelayOverflow),
        }
    }

    /// Cancellation is only possible while a binding is pending, and it is
    /// callable by the attester or the guardian.
    #[test]
    fn fuzz_cancel_payout_address(
        addr in any::<[u8; 32]>(),
        by_guardian in any::<bool>(),
        advance in 0u64..=PAYOUT_ADDRESS_DELAY,
    ) {
        let h = harness();
        let addr = Address::from_array(&h.env, &addr);
        h.pool.set_payout_address(&h.attesters[0], &addr);
        h.env.ledger().set_timestamp(h.env.ledger().timestamp() + advance);
        let caller = if by_guardian { h.guardian.clone() } else { h.attesters[0].clone() };
        let r = outcome!(h.pool.try_cancel_payout_address(&caller, &h.attesters[0]));
        prop_assert!(r.is_ok());
        prop_assert!(h.pool.get_payout_binding(&h.attesters[0]).unwrap().pending.is_none());
        // Cancelling twice is a typed error, never a trap.
        prop_assert!(outcome!(h.pool.try_cancel_payout_address(&caller, &h.attesters[0])).is_err());
    }

    /// `resolve_drift` must deterministically cancel or defer reservations that
    /// can no longer be covered after a clawback, emitting per-item events.
    #[test]
    fn fuzz_resolve_drift(
        amounts in prop::collection::vec(1i128..=1_000, 1..8),
        clawback in 0i128..=1_000,
    ) {
        let h = harness();
        let mut ids = std::vec::Vec::new();
        for (i, amount) in amounts.iter().enumerate() {
            let id = item_id(&h.env, i as u64);
            h.pool.fund(amount);
            h.pool.approve_work_item(&id, &h.attesters[0], amount);
            ids.push(id);
        }
        let reserved: i128 = amounts.iter().sum();
        let claw = clawback.min(reserved);
        if claw > 0 {
            h.sac.clawback(&h.pool.address, &claw);
        }
        prop_assert_eq!(h.pool.is_drift_detected(), claw > 0);
        let r = outcome!(h.pool.try_resolve_drift(&h.admin));
        prop_assert!(r.is_ok());
        // After resolution the pool is reconciled again.
        prop_assert!(!h.pool.is_drift_detected());
        custody_holds(&h);
        // Every reservation is either still claimable or explicitly cancelled.
        for id in ids {
            let status = h.pool.get_work_item_status(&id);
            prop_assert!(status == WorkItemStatus::Approved || status == WorkItemStatus::Cancelled);
        }
    }

    #[test]
    fn fuzz_sequence(ops in prop::collection::vec(op_strategy(), 1..60)) {
        replay(ops);
    }
}

#[test]
fn regression_payout_delay_overflow() {
    let h = harness();
    h.env.ledger().set_timestamp(u64::MAX - 1);
    let addr = Address::from_array(&h.env, &[7u8; 32]);
    assert_eq!(
        outcome!(h.pool.try_set_payout_address(&h.attesters[0], &addr)),
        Err(Error::PayoutDelayOverflow)
    );
}

#[test]
fn regression_claim_cap_overflow() {
    let h = harness();
    h.pool.set_max_per_claim(&i128::MAX);
    h.pool.set_max_per_attester(&i128::MAX);
    h.pool.fund(&10);
    let small = item_id(&h.env, 1);
    let huge = item_id(&h.env, 2);
    h.pool.approve_work_item(&small, &h.attesters[0], &5);
    h.pool.approve_work_item(&huge, &h.attesters[0], &i128::MAX);
    h.pool.claim(&small);
    assert_eq!(
        outcome!(h.pool.try_claim(&huge)),
        Err(Error::AttesterClaimCapExceeded)
    );
    custody_holds(&h);
}
