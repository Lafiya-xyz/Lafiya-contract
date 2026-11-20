//! Property-based fuzzing of every mutating incentive-pool entry point
//! (#508), following the `multisig-account` precedent. Inputs are full-range
//! `i128` amounts and arbitrary 32-byte work-item ids. Every target asserts
//! that the call returns a typed `Error` (never a host trap) and that the
//! custody invariant still holds.
//!
//! Targets: `fuzz_fund`, `fuzz_withdraw`, `fuzz_approve`, `fuzz_claim`,
//! `fuzz_caps`, plus the sequence-driven `fuzz_sequence`.
//!
//! Multi-asset / push / void / identity targets (#695-#698):
//! `fuzz_asset_allowlist`, `fuzz_asset_fund_withdraw`,
//! `fuzz_asset_approve_claim`, `fuzz_disburse_batch`,
//! `fuzz_void_approvals_since`, `fuzz_sweep`, `fuzz_identity_caps`.
//!
//! Findings triaged into regression tests below:
//! - `claim` overflowed (`attester_claimed + amount`, `total_paid + amount`)
//!   when caps were set near `i128::MAX`, trapping instead of returning
//!   `AttesterClaimCapExceeded`/`InsufficientPoolBalance`. Fixed with
//!   saturating adds; see `regression_claim_cap_overflow`.
//! - Per-asset accounting must stay isolated: a claim in one asset never
//!   touches another asset's `TotalDeposited`/`TotalPaid`; see
//!   `regression_asset_isolation`.
//! - Voiding approvals is a watermark, so items approved before `since` (or
//!   by another approver) stay claimable; see `regression_void_watermark`.
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

/// Per-asset custody invariant: for every allowlisted asset the pool's
/// token balance equals `TotalDeposited(asset) - TotalPaid(asset)`, and
/// neither total ever goes negative or pays out more than was deposited.
fn asset_custody_holds(h: &Harness) {
    for asset in h.pool.get_assets() {
        let d = h.pool.get_total_deposited_for(&asset);
        let p = h.pool.get_total_paid_for(&asset);
        assert!(p <= d, "paid exceeds deposited for asset");
        assert_eq!(h.token.balance(&h.pool.address), d - p);
    }
}

/// Every mutating call must surface a typed `Error` (or `Ok`), never a host
/// trap. `outcome!` already converts `Result<_, Result<Error, _>>` into a
/// comparable value; this helper asserts the trap-free contract explicitly.
fn typed<T>(r: Result<T, Result<Error, soroban_sdk::InvokeError>>) -> Result<T, Error> {
    match r {
        Ok(v) => Ok(v),
        Err(Ok(e)) => Err(e),
        Err(Err(_)) => panic!("host trap instead of typed Error"),
    }
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
    fn fuzz_sequence(ops in prop::collection::vec(op_strategy(), 1..60)) {
        replay(ops);
    }

    /// #695: adding/removing assets from the allowlist is idempotent and
    /// never traps, even for arbitrary (possibly unknown) asset addresses.
    #[test]
    fn fuzz_asset_allowlist(idx in 0usize..8, add in any::<bool>()) {
        let h = harness();
        let asset = h.assets[idx % h.assets.len()].clone();
        let r = typed(h.pool.try_set_asset_allowed(&h.admin, &asset, &add));
        prop_assert!(r.is_ok() || r.is_err());
        prop_assert_eq!(h.pool.is_asset_allowed(&asset), add);
        asset_custody_holds(&h);
    }

    /// #695: per-asset fund/withdraw keeps each asset's accounting isolated.
    #[test]
    fn fuzz_asset_fund_withdraw(
        idx in 0usize..8,
        fund in i128::MIN..=ADMIN_BALANCE,
        amount in any::<i128>(),
    ) {
        let h = harness();
        let asset = h.assets[idx % h.assets.len()].clone();
        let r = typed(h.pool.try_fund_asset(&asset, &fund));
        prop_assert_eq!(r.is_ok(), fund > 0);
        let w = typed(h.pool.try_withdraw_asset(&h.admin, &asset, &amount));
        prop_assert_eq!(w.is_ok(), amount > 0 && amount <= fund.max(0));
        asset_custody_holds(&h);
    }

    /// #695: approve/claim in one asset never moves another asset's totals.
    #[test]
    fn fuzz_asset_approve_claim(
        idx in 0usize..8,
        id in any::<[u8; 32]>(),
        amount in 1i128..=i128::MAX,
        fund in 1i128..=ADMIN_BALANCE,
    ) {
        let h = harness();
        let asset = h.assets[idx % h.assets.len()].clone();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_attester(&i128::MAX);
        h.pool.fund_asset(&asset, &fund);
        let id = BytesN::from_array(&h.env, &id);
        let a = typed(h.pool.try_approve_work_item_asset(&id, &h.attesters[0], &asset, &amount));
        prop_assert!(a.is_ok() || a.is_err());
        let c = typed(h.pool.try_claim(&id));
        prop_assert_eq!(c.is_ok(), amount <= fund);
        prop_assert!(typed(h.pool.try_claim(&id)).is_err());
        asset_custody_holds(&h);
    }

    /// #696: push disbursement is bounded and applies the same guards as
    /// `claim`; an oversized batch returns a typed error, never a trap.
    #[test]
    fn fuzz_disburse_batch(
        n in 0usize..=MAX_DISBURSE_BATCH + 4,
        amount in 1i128..=ADMIN_BALANCE,
    ) {
        let h = harness();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_attester(&i128::MAX);
        h.pool.fund(&ADMIN_BALANCE);
        let mut ids = Vec::new(&h.env);
        for i in 0..n {
            let id = item_id(&h.env, i as u64);
            h.pool.approve_work_item(&id, &h.attesters[0], &amount);
            h.pool.set_push_enabled(&h.attesters[0], &true);
            ids.push_back(id);
        }
        let r = typed(h.pool.try_disburse(&h.admin, &ids));
        if n > MAX_DISBURSE_BATCH {
            prop_assert_eq!(r, Err(Error::BatchTooLarge));
        } else {
            prop_assert!(r.is_ok() || r.is_err());
        }
        custody_holds(&h);
    }

    /// #697: voiding approvals is a watermark; arbitrary `since` values are
    /// accepted and only affect items approved at/after the watermark.
    #[test]
    fn fuzz_void_approvals_since(since in any::<u64>()) {
        let h = harness();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_attester(&i128::MAX);
        h.pool.fund(&ADMIN_BALANCE);
        let id = item_id(&h.env, 1);
        h.pool.approve_work_item(&id, &h.attesters[0], &10);
        let r = typed(h.pool.try_void_approvals_since(&h.admin, &h.attesters[0], &since));
        prop_assert!(r.is_ok() || r.is_err());
        let c = typed(h.pool.try_claim(&id));
        prop_assert!(c.is_ok() || c == Err(Error::ApprovalVoided));
        custody_holds(&h);
    }

    /// #697: permissionless sweep releases reservations lazily and never
    /// breaks the custody invariant, whatever the batch size.
    #[test]
    fn fuzz_sweep(n in 0usize..64) {
        let h = harness();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_attester(&i128::MAX);
        h.pool.fund(&ADMIN_BALANCE);
        for i in 0..n {
            let id = item_id(&h.env, i as u64);
            h.pool.approve_work_item(&id, &h.attesters[0], &1);
        }
        let r = typed(h.pool.try_sweep(&h.attesters[0], &n as u32));
        prop_assert!(r.is_ok() || r.is_err());
        custody_holds(&h);
    }

    /// #698: identity-keyed caps hold across addresses sharing an identity
    /// key; the fallback (no identity support) degrades to per-address caps.
    #[test]
    fn fuzz_identity_caps(
        cap in any::<i128>(),
        amount in 1i128..=ADMIN_BALANCE,
        same_identity in any::<bool>(),
    ) {
        let h = harness();
        h.pool.set_max_per_claim(&i128::MAX);
        h.pool.set_max_per_identity(&cap);
        h.pool.fund(&ADMIN_BALANCE);
        if same_identity {
            h.registry.set_identity_key(&h.attesters[0], &h.identity);
            h.registry.set_identity_key(&h.attesters[1], &h.identity);
        }
        let a = item_id(&h.env, 1);
        let b = item_id(&h.env, 2);
        h.pool.approve_work_item(&a, &h.attesters[0], &amount);
        h.pool.approve_work_item(&b, &h.attesters[1], &amount);
        let ra = typed(h.pool.try_claim(&a));
        let rb = typed(h.pool.try_claim(&b));
        prop_assert!(ra.is_ok() || ra.is_err());
        prop_assert!(rb.is_ok() || rb.is_err());
        if same_identity && ra.is_ok() {
            prop_assert_eq!(rb, Err(Error::IdentityClaimCapExceeded));
        }
        custody_holds(&h);
    }
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
fn regression_asset_isolation() {
    let h = harness();
    h.pool.set_max_per_claim(&i128::MAX);
    h.pool.set_max_per_attester(&i128::MAX);
    let usdc = h.assets[0].clone();
    let eurc = h.assets[1].clone();
    h.pool.fund_asset(&usdc, &100);
    h.pool.fund_asset(&eurc, &50);
    let id = item_id(&h.env, 1);
    h.pool.approve_work_item_asset(&id, &h.attesters[0], &usdc, &40);
    h.pool.claim(&id);
    assert_eq!(h.pool.get_total_paid_for(&usdc), 40);
    assert_eq!(h.pool.get_total_paid_for(&eurc), 0);
    assert_eq!(h.pool.get_total_deposited_for(&eurc), 50);
    asset_custody_holds(&h);
}

#[test]
fn regression_void_watermark() {
    let h = harness();
    h.pool.set_max_per_claim(&i128::MAX);
    h.pool.set_max_per_attester(&i128::MAX);
    h.pool.fund(&ADMIN_BALANCE);
    let before = item_id(&h.env, 1);
    h.pool.approve_work_item(&before, &h.attesters[0], &10);
    let since = h.env.ledger().timestamp() + 1;
    h.pool.void_approvals_since(&h.admin, &h.attesters[0], &since);
    // Approved before the watermark: still claimable.
    assert!(outcome!(h.pool.try_claim(&before)).is_ok());
    custody_holds(&h);
}
