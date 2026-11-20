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
//! - vesting grants vest linearly after their cliff, never pay more than
//!   `total`, and reserved liabilities (unvested + unclaimed grant amounts)
//!   are never drained by `withdraw`
//!
//! Finding: approvals do not reserve funds, so `withdraw` can drop custody
//! below outstanding (approved, unclaimed) liabilities. Those claims then
//! fail with `InsufficientPoolBalance` until the pool is re-funded. This is
//! by design (approvals are promises, not escrow) and the harness asserts
//! that behaviour rather than flagging it. Vesting grants, by contrast, *are*
//! reserved: `withdraw` must leave enough custody to cover every grant's
//! unclaimed remainder.
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
pub const GRANTS: usize = 4;
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
    CreateGrant {
        attester: u8,
        total: i128,
        cliff: u64,
        duration: u64,
    },
    ClaimVested(u8),
    Forfeit(u8),
    AdvanceTime(u64),
}

pub fn op_strategy() -> impl Strategy<Value = Op> {
    let amt = -10i128..=600;
    let item = 0..ITEMS;
    let att = 0..ATTESTERS as u8;
    let grant = 0..GRANTS as u8;
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
        2 => (att.clone(), 1i128..=600, 0u64..=50, 1u64..=200)
            .prop_map(|(attester, total, cliff, duration)| Op::CreateGrant {
                attester,
                total,
                cliff,
                duration,
            }),
        3 => grant.clone().prop_map(Op::ClaimVested),
        1 => grant.prop_map(Op::Forfeit),
        2 => (0u64..=100).prop_map(Op::AdvanceTime),
    ]
}

/// Model of a single vesting grant. `start` is the ledger timestamp at
/// creation; `claimed` and `forfeited` track released amounts.
#[derive(Clone, Debug)]
struct GrantModel {
    attester: usize,
    total: i128,
    start: u64,
    cliff: u64,
    duration: u64,
    claimed: i128,
    forfeited: bool,
}

impl GrantModel {
    /// Amount vested at time `now` (linear after the cliff, saturating at
    /// `total`). Uses the same integer math as the contract so rounding is
    /// exercised identically.
    fn vested(&self, now: u64) -> i128 {
        let elapsed = now.saturating_sub(self.start);
        if elapsed < self.cliff {
            return 0;
        }
        let after = elapsed - self.cliff;
        if after >= self.duration {
            return self.total;
        }
        // total * after / duration, floor division.
        (self.total * after as i128) / self.duration as i128
    }

    /// Vested-but-unclaimed amount at time `now`.
    fn claimable(&self, now: u64) -> i128 {
        if self.forfeited {
            return 0;
        }
        self.vested(now) - self.claimed
    }

    /// Unclaimed remainder still reserved against the pool (unvested +
    /// vested-but-unclaimed), or zero once forfeited.
    fn reserved(&self) -> i128 {
        if self.forfeited {
            0
        } else {
            self.total - self.claimed
        }
    }
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
    now: u64,
    grants: BTreeMap<u8, GrantModel>,
}

impl Model {
    /// Sum of reserved vesting liabilities across all live grants.
    fn reserved_vesting(&self) -> i128 {
        self.grants.values().map(GrantModel::reserved).sum()
    }
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
    let pool_id = env.register(IncentivePool, ());
    let pool = IncentivePoolClient::new(&env, &pool_id);
    pool.initialize(
        &admin,
        &Address::generate(&env),
        &token_id,
        &reg_id,
        &START_CAP,
        &START_CAP,
    );
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
                // Reserved vesting liabilities must remain covered.
                if a > self.deposited - self.paid - self.reserved_vesting() {
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
            Op::CreateGrant {
                attester,
                total,
                cliff,
                duration,
            } => {
                paused()?;
                if total <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                if duration == 0 {
                    return Err(Error::NonPositiveAmount);
                }
                if self.suspended[attester as usize] {
                    return Err(Error::AttesterNotAllowlisted);
                }
                // Grants are reserved, so the pool must already hold enough
                // unreserved custody to back the new liability.
                if total > self.deposited - self.paid - self.reserved_vesting() {
                    return Err(Error::InsufficientPoolBalance);
                }
                let id = self.grants.len() as u8;
                self.grants.insert(
                    id,
                    GrantModel {
                        attester: attester as usize,
                        total,
                        start: self.now,
                        cliff,
                        duration,
                        claimed: 0,
                        forfeited: false,
                    },
                );
            }
            Op::ClaimVested(id) => {
                paused()?;
                let g = self.grants.get(&id).ok_or(Error::GrantNotFound)?;
                if g.forfeited {
                    return Err(Error::GrantForfeited);
                }
                if self.suspended[g.attester] {
                    return Err(Error::AttesterNotAllowlisted);
                }
                let amount = g.claimable(self.now);
                if amount <= 0 {
                    return Err(Error::NothingToClaim);
                }
                let att = g.attester;
                let g = self.grants.get_mut(&id).unwrap();
                g.claimed += amount;
                self.per_attester[att] += amount;
                self.paid += amount;
            }
            Op::Forfeit(id) => {
                paused()?;
                let g = self.grants.get(&id).ok_or(Error::GrantNotFound)?;
                if g.forfeited {
                    return Err(Error::GrantForfeited);
                }
                let g = self.grants.get_mut(&id).unwrap();
                g.forfeited = true;
            }
            Op::AdvanceTime(dt) => {
                self.now = self.now.saturating_add(dt);
            }
        }
        Ok(())
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
            Op::CreateGrant {
                attester,
                total,
                cliff,
                duration,
            } => outcome!(h.pool.try_create_vesting_grant(
                &h.attesters[attester as usize],
                &total,
                &cliff,
                &duration
            )),
            Op::ClaimVested(id) => outcome!(h.pool.try_claim_vested(&id)),
            Op::Forfeit(id) => outcome!(h.pool.try_forfeit(&h.admin, &id)),
            Op::AdvanceTime(dt) => {
                let now = h.env.ledger().timestamp();
                h.env.ledger().set_timestamp(now.saturating_add(dt));
                Ok(())
            }
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
        }

        // Vesting invariants: reserved liabilities are reported by the pool
        // and never exceed custody; each grant's claimed total matches the
        // model and never exceeds its total.
        let reserved = h.pool.get_reserved_liabilities();
        assert_eq!(
            reserved,
            m.reserved_vesting(),
            "step {n}: reserved liabilities"
        );
        assert!(
            reserved <= deposited - paid,
            "step {n}: reserved liabilities exceed custody"
        );
        for (id, g) in m.grants.iter() {
            let on_chain = h.pool.get_vesting_grant(id);
            assert_eq!(on_chain.claimed, g.claimed, "step {n}: grant {id} claimed");
            assert_eq!(
                on_chain.forfeited, g.forfeited,
                "step {n}: grant {id} forfeited"
            );
            assert!(
                on_chain.claimed <= on_chain.total,
                "step {n}: grant {id} over-claimed"
            );
        }
    }
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

    /// Focused rounding check: a grant whose `total` does not divide evenly
    /// by `duration` must never over-pay, and the final claim must release
    /// exactly the remainder.
    #[test]
    fn vesting_rounding_never_overpays(
        total in 1i128..=1_000,
        duration in 1u64..=97,
        steps in prop::collection::vec(0u64..=20, 1..30),
    ) {
        let h = harness();
        h.pool.fund(&(total + 1));
        let att = h.attesters[0].clone();
        let id = h.pool.create_vesting_grant(&att, &total, &0, &duration);
        let mut paid: i128 = 0;
        for dt in steps {
            let now = h.env.ledger().timestamp();
            h.env.ledger().set_timestamp(now.saturating_add(dt));
            let elapsed = h.env.ledger().timestamp();
            let expected_vested = if elapsed >= duration {
                total
            } else {
                (total * elapsed as i128) / duration as i128
            };
            let _ = h.pool.try_claim_vested(&id);
            let g = h.pool.get_vesting_grant(&id);
            assert!(g.claimed <= expected_vested, "over-paid vested amount");
            assert!(g.claimed <= total, "claimed exceeds total");
            paid = g.claimed;
        }
        // Jump past the end and drain the remainder.
        let now = h.env.ledger().timestamp();
        h.env.ledger().set_timestamp(now.saturating_add(duration));
        let _ = h.pool.try_claim_vested(&id);
        let g = h.pool.get_vesting_grant(&id);
        assert_eq!(g.claimed, total, "final claim must release the remainder");
        assert!(g.claimed >= paid);
    }
}
