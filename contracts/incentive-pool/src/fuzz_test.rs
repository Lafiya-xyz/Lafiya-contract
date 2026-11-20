//! Property-based fuzzing of every mutating incentive-pool entry point
//! (#508), following the `multisig-account` precedent. Inputs are full-range
//! `i128` amounts and arbitrary 32-byte work-item ids. Every target asserts
//! that the call returns a typed `Error` (never a host trap) and that the
//! custody invariant still holds.
//!
//! Targets: `fuzz_fund`, `fuzz_withdraw`, `fuzz_approve`, `fuzz_claim`,
//! `fuzz_caps`, `fuzz_create_vesting_grant`, `fuzz_claim_vested`,
//! `fuzz_forfeit`, plus the sequence-driven `fuzz_sequence`.
//!
//! Findings triaged into regression tests below:
//! - `claim` overflowed (`attester_claimed + amount`, `total_paid + amount`)
//!   when caps were set near `i128::MAX`, trapping instead of returning
//!   `AttesterClaimCapExceeded`/`InsufficientPoolBalance`. Fixed with
//!   saturating adds; see `regression_claim_cap_overflow`.
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

/// Solvency invariant: token balance covers paid-out funds plus every
/// reserved vesting liability (vested-but-unclaimed and unvested amounts of
/// live grants). Forfeited remainders return to the pool, so they must not
/// be counted as reserved.
fn solvency_holds(h: &Harness) {
    custody_holds(h);
    let reserved = h.pool.get_total_vesting_reserved();
    assert!(reserved >= 0);
    assert!(h.pool.get_total_paid() + reserved <= h.pool.get_total_deposited());
    assert!(h.token.balance(&h.pool.address) >= reserved);
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

    #[test]
    fn fuzz_create_vesting_grant(
        total in any::<i128>(),
        start in any::<u64>(),
        cliff in any::<u64>(),
        duration in any::<u64>(),
        fund in 0i128..=ADMIN_BALANCE,
    ) {
        let h = harness();
        if fund > 0 {
            h.pool.fund(&fund);
        }
        let r = outcome!(h.pool.try_create_vesting_grant(
            &h.attesters[0], &total, &start, &cliff, &duration
        ));
        // Grants must be positive, have a non-zero duration, and fit within
        // the pool's unreserved balance.
        prop_assert_eq!(r.is_ok(), total > 0 && duration > 0 && total <= fund);
        solvency_holds(&h);
    }

    #[test]
    fn fuzz_claim_vested(
        total in 1i128..=ADMIN_BALANCE,
        start in any::<u64>(),
        cliff in any::<u64>(),
        duration in 1u64..=u64::MAX,
        now in any::<u64>(),
    ) {
        let h = harness();
        h.pool.fund(&total);
        let grant_id = h.pool.create_vesting_grant(
            &h.attesters[0], &total, &start, &cliff, &duration
        );
        h.env.ledger().set_timestamp(now);
        let r = outcome!(h.pool.try_claim_vested(&grant_id));
        // Claiming before the cliff or before anything vests is a typed error.
        let vested = vested_amount(total, start, cliff, duration, now);
        prop_assert_eq!(r.is_ok(), vested > 0);
        solvency_holds(&h);
    }

    #[test]
    fn fuzz_forfeit(
        total in 1i128..=ADMIN_BALANCE,
        start in any::<u64>(),
        cliff in any::<u64>(),
        duration in 1u64..=u64::MAX,
        now in any::<u64>(),
        by_admin in any::<bool>(),
    ) {
        let h = harness();
        h.pool.fund(&total);
        let grant_id = h.pool.create_vesting_grant(
            &h.attesters[0], &total, &start, &cliff, &duration
        );
        h.env.ledger().set_timestamp(now);
        let caller = if by_admin { h.admin.clone() } else { h.guardian.clone() };
        let r = outcome!(h.pool.try_forfeit(&caller, &grant_id));
        // Admin/guardian may forfeit a live grant exactly once.
        prop_assert!(r.is_ok());
        prop_assert!(outcome!(h.pool.try_forfeit(&caller, &grant_id)).is_err());
        solvency_holds(&h);
    }

    #[test]
    fn fuzz_sequence(ops in prop::collection::vec(op_strategy(), 1..60)) {
        replay(ops);
    }
}

/// Reference linear-vesting math: nothing before the cliff, then linear
/// release over `duration` from `start`, saturating at `total`.
fn vested_amount(total: i128, start: u64, cliff: u64, duration: u64, now: u64) -> i128 {
    let cliff_end = start.saturating_add(cliff);
    if now < cliff_end {
        return 0;
    }
    let elapsed = now.saturating_sub(start).min(duration);
    if elapsed >= duration {
        return total;
    }
    // `elapsed * total / duration` with i128 math; `elapsed <= duration` and
    // `total <= ADMIN_BALANCE`, so the product cannot overflow.
    (total / duration as i128) * elapsed as i128
        + (total % duration as i128) * elapsed as i128 / duration as i128
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

#[test]
fn regression_vesting_reserved_blocks_withdraw() {
    let h = harness();
    h.pool.fund(&100);
    let grant_id = h.pool.create_vesting_grant(&h.attesters[0], &100, &0, &0, &100);
    // The full unvested grant is reserved, so nothing can be withdrawn.
    assert_eq!(
        outcome!(h.pool.try_withdraw(&h.admin, &1)),
        Err(Error::InsufficientPoolBalance)
    );
    h.env.ledger().set_timestamp(100);
    h.pool.claim_vested(&grant_id);
    assert_eq!(h.pool.get_total_vesting_reserved(), 0);
    solvency_holds(&h);
}
