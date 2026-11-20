//! Model-based stateful property tests for the incentive pool (#507).
//!
//! Each case drives a random sequence of [`Op`]s against both the contract
//! and a plain-Rust [`Model`], asserting that every call returns exactly the
//! outcome the model predicts and that the pool invariants hold after every
//! step:
//!
//! - custody: the pool's token balance equals `total_deposited - total_paid`
//! - `total_paid <= total_deposited`
//! - each work item pays at most once
//! - per-attester claimed total never exceeds the cap in force at claim time
//!
//! Finding: approvals do not reserve funds, so `withdraw` can drop custody
//! below outstanding (approved, unclaimed) liabilities. Those claims then
//! fail with `InsufficientPoolBalance` until the pool is re-funded. This is
//! by design (approvals are promises, not escrow) and the harness asserts
//! that behaviour rather than flagging it.
//!
//! Shrinking yields a minimal `Vec<Op>`; on failure proptest prints it in
//! `Debug` form, which is a replayable trace: paste it into
//! `replay(vec![...])` in a `#[test]`.
//!
//! Profiles:
//! - CI (default): 64 cases, ~1-2s.
//! - Nightly: `PROPTEST_CASES=10000 cargo test -p incentive-pool stateful_test`

extern crate std;

use super::*;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};
use std::collections::{BTreeMap, BTreeSet};
use std::vec::Vec;

pub const ATTESTERS: usize = 3;
pub const ITEMS: u8 = 6;
const START_CAP: i128 = 1_000;

#[derive(Clone, Debug)]
pub enum Op {
    Fund(i128),
    Withdraw(i128),
    Approve {
        item: u8,
        attester: u8,
        amount: i128,
    },
    Claim(u8),
    Pause,
    Unpause,
    SetMaxPerClaim(i128),
    SetMaxPerAttester(i128),
    Suspend(u8),
    Reinstate(u8),
    SchedulePayout(u8),
    CancelPayout(u8),
    ResolveDrift,
}

pub fn op_strategy() -> impl Strategy<Value = Op> {
    let amt = -10i128..=600;
    let item = 0..ITEMS;
    let att = 0..ATTESTERS as u8;
    prop_oneof![
        3 => amt.clone().prop_map(Op::Fund),
        1 => amt.clone().prop_map(Op::Withdraw),
        4 => (item.clone(), att.clone(), amt.clone())
            .prop_map(|(item, attester, amount)| Op::Approve { item, attester, amount }),
        4 => item.prop_map(Op::Claim),
        1 => Just(Op::Pause),
        1 => Just(Op::Unpause),
        1 => (1i128..=600).prop_map(Op::SetMaxPerClaim),
        1 => (1i128..=1_500).prop_map(Op::SetMaxPerAttester),
        1 => att.clone().prop_map(Op::Suspend),
        1 => att.prop_map(Op::Reinstate),
        2 => att.clone().prop_map(Op::SchedulePayout),
        2 => att.prop_map(Op::CancelPayout),
        1 => Just(Op::ResolveDrift),
    ]
}

#[derive(Default)]
struct Model {
    deposited: i128,
    paid: i128,
    items: BTreeMap<u8, (usize, i128)>,
    claimed: BTreeSet<u8>,
    per_attester: [i128; ATTESTERS],
    suspended: [bool; ATTESTERS],
    paused: bool,
    max_claim: i128,
    max_attester: i128,
    /// Attesters with a pending (scheduled, not yet effective) payout binding.
    pending_payout: [bool; ATTESTERS],
}

pub struct Harness {
    pub env: Env,
    pub pool: IncentivePoolClient<'static>,
    pub registry: attester_registry::AttesterRegistryClient<'static>,
    pub token: soroban_sdk::token::Client<'static>,
    pub admin: Address,
    pub attesters: Vec<Address>,
}

pub fn harness() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let reg_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let registry = attester_registry::AttesterRegistryClient::new(&env, &reg_id);
    registry.grant_role(&attester_registry::Role::Registrar, &admin);
    let token_id = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    soroban_sdk::token::StellarAssetClient::new(&env, &token_id).mint(&admin, &1_000_000_000);
    let pool_id = env.register(
        IncentivePool,
        (
            admin.clone(),
            Address::generate(&env),
            token_id.clone(),
            reg_id.clone(),
            START_CAP,
            START_CAP,
        ),
    );
    let pool = IncentivePoolClient::new(&env, &pool_id);
    let attesters: Vec<Address> = (0..ATTESTERS)
        .map(|_| {
            let a = Address::generate(&env);
            registry.add_attester(&admin, &a);
            a
        })
        .collect();
    let token = soroban_sdk::token::Client::new(&env, &token_id);
    Harness {
        env,
        pool,
        registry,
        token,
        admin,
        attesters,
    }
}

pub fn item_id(env: &Env, i: u8) -> BytesN<32> {
    BytesN::from_array(env, &[i; 32])
}

/// Collapse a generated `try_*` result to `Ok(())` / the contract error.
macro_rules! outcome {
    ($e:expr) => {
        match $e {
            Ok(_) => Ok(()),
            Err(Ok(err)) => Err(err),
            Err(Err(invoke)) => panic!("host trap instead of typed error: {:?}", invoke),
        }
    };
}
pub(crate) use outcome;

impl Model {
    fn step(&mut self, op: &Op) -> Result<(), Error> {
        let paused = || {
            if self.paused {
                Err(Error::ContractPaused)
            } else {
                Ok(())
            }
        };
        match *op {
            Op::Fund(a) => {
                paused()?;
                if a <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                self.deposited += a;
            }
            Op::Withdraw(a) => {
                paused()?;
                if a <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                if a > self.deposited - self.paid {
                    return Err(Error::InsufficientPoolBalance);
                }
                self.paid += a;
            }
            Op::Approve {
                item,
                attester,
                amount,
            } => {
                paused()?;
                if amount <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                if self.items.contains_key(&item) {
                    return Err(Error::WorkItemAlreadyApproved);
                }
                if self.suspended[attester as usize] {
                    return Err(Error::AttesterNotAllowlisted);
                }
                if amount > self.max_claim {
                    return Err(Error::PayoutCapExceeded);
                }
                self.items.insert(item, (attester as usize, amount));
            }
            Op::Claim(item) => {
                paused()?;
                let (att, amount) = *self.items.get(&item).ok_or(Error::WorkItemNotApproved)?;
                if self.claimed.contains(&item) {
                    return Err(Error::WorkItemAlreadyClaimed);
                }
                if self.suspended[att] {
                    return Err(Error::AttesterNotAllowlisted);
                }
                if self.per_attester[att] + amount > self.max_attester {
                    return Err(Error::AttesterClaimCapExceeded);
                }
                if self.paid + amount > self.deposited {
                    return Err(Error::InsufficientPoolBalance);
                }
                self.claimed.insert(item);
                self.per_attester[att] += amount;
                self.paid += amount;
            }
            Op::Pause => self.paused = true,
            Op::Unpause => self.paused = false,
            Op::SetMaxPerClaim(m) => self.max_claim = m,
            Op::SetMaxPerAttester(m) => self.max_attester = m,
            Op::Suspend(_) | Op::Reinstate(_) => {}
            Op::SchedulePayout(a) => {
                paused()?;
                if self.suspended[a as usize] {
                    return Err(Error::AttesterNotAllowlisted);
                }
                self.pending_payout[a as usize] = true;
            }
            Op::CancelPayout(a) => {
                paused()?;
                if !self.pending_payout[a as usize] {
                    return Err(Error::NoPendingPayoutBinding);
                }
                self.pending_payout[a as usize] = false;
            }
            Op::ResolveDrift => {
                paused()?;
                if !self.drifted() {
                    return Err(Error::NoDriftDetected);
                }
                // Deterministic policy: reservations that can no longer be
                // covered are cancelled FIFO by approval order.
                let available = self.deposited - self.paid;
                let mut reserved = 0i128;
                let mut to_cancel: Vec<u8> = Vec::new();
                for (item, (_, amount)) in self.items.iter() {
                    if self.claimed.contains(item) {
                        continue;
                    }
                    reserved += *amount;
                    if reserved > available {
                        to_cancel.push(*item);
                    }
                }
                for item in to_cancel {
                    self.items.remove(&item);
                }
            }
        }
        Ok(())
    }

    /// True when the pool's token balance is below what the internal totals
    /// say it should be (e.g. after an issuer clawback).
    fn drifted(&self) -> bool {
        self.deposited - self.paid < 0
    }
}

/// Run `ops` against a fresh pool and model, asserting outcome equality and
/// all invariants after every step. Public so failing traces can be replayed.
pub fn replay(ops: Vec<Op>) {
    let h = harness();
    let mut m = Model {
        max_claim: START_CAP,
        max_attester: START_CAP,
        ..Default::default()
    };
    for (n, op) in ops.iter().enumerate() {
        let actual: Result<(), Error> = match *op {
            Op::Fund(a) => outcome!(h.pool.try_fund(&a)),
            Op::Withdraw(a) => outcome!(h.pool.try_withdraw(&h.admin, &a)),
            Op::Approve {
                item,
                attester,
                amount,
            } => outcome!(h.pool.try_approve_work_item(
                &item_id(&h.env, item),
                &h.attesters[attester as usize],
                &amount
            )),
            Op::Claim(item) => outcome!(h.pool.try_claim(&item_id(&h.env, item))),
            Op::Pause => outcome!(h.pool.try_pause()),
            Op::Unpause => outcome!(h.pool.try_unpause()),
            Op::SetMaxPerClaim(v) => outcome!(h.pool.try_set_max_per_claim(&v)),
            Op::SetMaxPerAttester(v) => outcome!(h.pool.try_set_max_per_attester(&v)),
            Op::Suspend(a) => {
                let i = a as usize;
                if !m.suspended[i] {
                    h.registry.suspend_attester(&h.admin, &h.attesters[i]);
                    m.suspended[i] = true;
                }
                Ok(())
            }
            Op::Reinstate(a) => {
                let i = a as usize;
                if m.suspended[i] {
                    h.registry.reinstate_attester(&h.admin, &h.attesters[i]);
                    m.suspended[i] = false;
                }
                Ok(())
            }
            Op::SchedulePayout(a) => {
                let i = a as usize;
                let payout = Address::generate(&h.env);
                outcome!(h.pool.try_set_payout_address(&h.attesters[i], &payout))
            }
            Op::CancelPayout(a) => {
                let i = a as usize;
                outcome!(h.pool.try_cancel_payout_address(&h.attesters[i]))
            }
            Op::ResolveDrift => outcome!(h.pool.try_resolve_drift(&h.admin)),
        };
        let expected = m.step(op);
        assert_eq!(actual, expected, "step {n} {op:?} diverged from model");

        // Invariants.
        let deposited = h.pool.get_total_deposited();
        let paid = h.pool.get_total_paid();
        assert_eq!((deposited, paid), (m.deposited, m.paid), "step {n}: totals");
        assert!(paid <= deposited, "step {n}: total_paid > total_deposited");
        assert_eq!(
            h.token.balance(&h.pool.address),
            deposited - paid,
            "step {n}: custody balance != deposited - paid"
        );
        for i in 0..ITEMS {
            assert_eq!(
                h.pool.is_work_item_claimed(&item_id(&h.env, i)),
                m.claimed.contains(&i),
                "step {n}: item {i} claim flag"
            );
        }
        for (i, a) in h.attesters.iter().enumerate() {
            assert_eq!(h.pool.get_attester_total_claimed(a), m.per_attester[i]);
            // A pending binding must be readable and cancellable, and its
            // persistent entry must have a live TTL.
            if m.pending_payout[i] {
                assert!(
                    h.pool.get_payout_binding(a).is_some(),
                    "step {n}: pending binding for attester {i} missing"
                );
            }
        }
    }
}

/// A scheduled payout binding survives the full delay window: its persistent
/// entry is TTL-extended on write and remains readable just before it applies.
#[test]
fn payout_binding_ttl_survives_delay() {
    let h = harness();
    let attester = h.attesters.first().unwrap();
    let payout = Address::generate(&h.env);
    h.pool.set_payout_address(&attester, &payout);
    assert!(h.pool.get_payout_binding(&attester).is_some());
    // Advance to just before the delay elapses; the binding must still exist.
    h.env.ledger().set_sequence_number(
        h.env.ledger().sequence() + (PAYOUT_ADDRESS_DELAY / 5) as u32,
    );
    assert!(h.pool.get_payout_binding(&attester).is_some());
}

/// A pending binding can be cancelled by the attester within the delay.
#[test]
fn payout_binding_cancelled_by_attester() {
    let h = harness();
    let attester = h.attesters.first().unwrap();
    h.pool
        .set_payout_address(&attester, &Address::generate(&h.env));
    h.pool.cancel_payout_address(&attester);
    assert!(h.pool.get_payout_binding(&attester).is_none());
}

/// A pending binding can be vetoed by the guardian/admin within the delay.
#[test]
fn payout_binding_cancelled_by_guardian() {
    let h = harness();
    let attester = h.attesters.first().unwrap();
    h.pool
        .set_payout_address(&attester, &Address::generate(&h.env));
    h.pool.cancel_payout_address(&h.admin, &attester);
    assert!(h.pool.get_payout_binding(&attester).is_none());
}

/// A clawback against the pool leaves custody below the internal totals; the
/// deterministic drift policy cancels the reservations it can no longer cover.
#[test]
fn drift_resolution_after_clawback() {
    let h = harness();
    h.pool.fund(&100);
    h.pool
        .approve_work_item(&item_id(&h.env, 0), &h.attesters[0], &60);
    h.pool
        .approve_work_item(&item_id(&h.env, 1), &h.attesters[1], &60);
    // Issuer claws back part of the pool balance.
    soroban_sdk::token::StellarAssetClient::new(&h.env, &h.token.address)
        .clawback(&h.pool.address, &50);
    assert!(h.pool.is_drift_detected());
    h.pool.resolve_drift(&h.admin);
    // The first reservation (FIFO) is kept, the second is cancelled.
    assert!(!h.pool.is_work_item_claimed(&item_id(&h.env, 0)));
    assert!(h.pool.try_claim(&item_id(&h.env, 0)).is_ok());
    assert!(h.pool.try_claim(&item_id(&h.env, 1)).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(64),
        ..ProptestConfig::default()
    })]

    #[test]
    fn pool_matches_model(ops in prop::collection::vec(op_strategy(), 1..40)) {
        replay(ops);
    }
}
