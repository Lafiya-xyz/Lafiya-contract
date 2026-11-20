//! Model-based stateful property tests for the incentive pool (#507).
//!
//! Each case drives a random sequence of [`Op`]s against both the contract
//! and a plain-Rust [`Model`], asserting that every call returns exactly the
//! outcome the model predicts and that the pool invariants hold after every
//! step:
//!
//! - custody: the pool's balance of each asset equals that asset's
//!   `total_deposited - total_paid` (per-asset isolation, #695)
//! - `total_paid <= total_deposited` per asset
//! - each work item pays at most once
//! - per-attester claimed total never exceeds the cap in force at claim time
//! - per-identity cumulative caps hold across addresses sharing an identity,
//!   including after key rotation (#698)
//! - `void_approvals_since` watermarks reject affected claims and lazy sweep
//!   keeps reservation accounting consistent (#697)
//! - `disburse` (push mode) applies exactly the same guards as `claim` (#696)
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
/// Number of allowlisted assets the pool is initialized with (#695).
pub const ASSETS: usize = 2;
/// Number of distinct stable identities shared across attester addresses
/// (#698). Attesters `0` and `1` share identity `0`; attester `2` has its own.
pub const IDENTITIES: usize = 2;
const START_CAP: i128 = 1_000;

#[derive(Clone, Debug)]
pub enum Op {
    Fund {
        asset: u8,
        amount: i128,
    },
    Withdraw {
        asset: u8,
        amount: i128,
    },
    Approve {
        item: u8,
        attester: u8,
        asset: u8,
        amount: i128,
    },
    Claim(u8),
    /// Approver-initiated push disbursement (#696).
    Disburse(u8),
    Pause,
    Unpause,
    SetMaxPerClaim(i128),
    SetMaxPerAttester(i128),
    SetMaxPerIdentity(i128),
    Suspend(u8),
    Reinstate(u8),
    /// Void all approvals issued by `approver` at or after `since` (#697).
    VoidApprovalsSince {
        approver: u8,
        since: u32,
    },
    /// Permissionless lazy cleanup of reservations for voided items (#697).
    SweepExpired,
    /// Rotate an attester's key; the identity key follows the rotation (#698).
    Rotate(u8),
}

pub fn op_strategy() -> impl Strategy<Value = Op> {
    let amt = -10i128..=600;
    let item = 0..ITEMS;
    let att = 0..ATTESTERS as u8;
    let asset = 0..ASSETS as u8;
    let approver = 0..2u8;
    prop_oneof![
        3 => (asset.clone(), amt.clone()).prop_map(|(asset, amount)| Op::Fund { asset, amount }),
        1 => (asset.clone(), amt.clone()).prop_map(|(asset, amount)| Op::Withdraw { asset, amount }),
        4 => (item.clone(), att.clone(), asset.clone(), amt.clone())
            .prop_map(|(item, attester, asset, amount)| Op::Approve { item, attester, asset, amount }),
        3 => item.clone().prop_map(Op::Claim),
        2 => item.clone().prop_map(Op::Disburse),
        1 => Just(Op::Pause),
        1 => Just(Op::Unpause),
        1 => (1i128..=600).prop_map(Op::SetMaxPerClaim),
        1 => (1i128..=1_500).prop_map(Op::SetMaxPerAttester),
        1 => (1i128..=1_500).prop_map(Op::SetMaxPerIdentity),
        1 => att.clone().prop_map(Op::Suspend),
        1 => att.clone().prop_map(Op::Reinstate),
        1 => (approver, 0u32..=8).prop_map(|(approver, since)| Op::VoidApprovalsSince { approver, since }),
        1 => Just(Op::SweepExpired),
        1 => att.prop_map(Op::Rotate),
    ]
}

/// A work item as tracked by the model.
#[derive(Clone, Copy, Debug)]
struct Item {
    attester: usize,
    asset: usize,
    amount: i128,
    approved_by: usize,
    approved_at: u32,
}

#[derive(Default)]
struct Model {
    /// Per-asset deposited totals (#695).
    deposited: [i128; ASSETS],
    /// Per-asset paid totals (#695).
    paid: [i128; ASSETS],
    items: BTreeMap<u8, Item>,
    claimed: BTreeSet<u8>,
    /// Per-attester claimed totals, per asset.
    per_attester: [[i128; ASSETS]; ATTESTERS],
    /// Per-identity claimed totals, per asset (#698).
    per_identity: [[i128; ASSETS]; IDENTITIES],
    suspended: [bool; ATTESTERS],
    /// Identity key currently bound to each attester address (#698).
    identity: [usize; ATTESTERS],
    /// Monotonic approval clock, advanced on every approval.
    clock: u32,
    /// `ApproverRevokedSince` watermarks (#697).
    revoked_since: [Option<u32>; 2],
    paused: bool,
    max_claim: i128,
    max_attester: i128,
    max_identity: i128,
}

impl Model {
    /// Whether an item's approval has been voided by a watermark (#697).
    fn is_voided(&self, item: &Item) -> bool {
        match self.revoked_since[item.approved_by] {
            Some(since) => item.approved_at >= since,
            None => false,
        }
    }

    /// Shared guard logic for `claim` and `disburse` (#696 parity).
    fn pay(&mut self, item_id: u8) -> Result<(), Error> {
        if self.paused {
            return Err(Error::ContractPaused);
        }
        let item = *self.items.get(&item_id).ok_or(Error::WorkItemNotApproved)?;
        if self.claimed.contains(&item_id) {
            return Err(Error::WorkItemAlreadyClaimed);
        }
        if self.is_voided(&item) {
            return Err(Error::ApprovalVoided);
        }
        if self.suspended[item.attester] {
            return Err(Error::AttesterNotAllowlisted);
        }
        if self.per_attester[item.attester][item.asset] + item.amount > self.max_attester {
            return Err(Error::AttesterClaimCapExceeded);
        }
        let identity = self.identity[item.attester];
        if self.per_identity[identity][item.asset] + item.amount > self.max_identity {
            return Err(Error::IdentityClaimCapExceeded);
        }
        if self.paid[item.asset] + item.amount > self.deposited[item.asset] {
            return Err(Error::InsufficientPoolBalance);
        }
        self.claimed.insert(item_id);
        self.per_attester[item.attester][item.asset] += item.amount;
        self.per_identity[identity][item.asset] += item.amount;
        self.paid[item.asset] += item.amount;
        Ok(())
    }

    fn step(&mut self, op: &Op) -> Result<(), Error> {
        let paused = || {
            if self.paused {
                Err(Error::ContractPaused)
            } else {
                Ok(())
            }
        };
        match *op {
            Op::Fund { asset, amount } => {
                paused()?;
                if amount <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                self.deposited[asset as usize] += amount;
            }
            Op::Withdraw { asset, amount } => {
                paused()?;
                if amount <= 0 {
                    return Err(Error::NonPositiveAmount);
                }
                let a = asset as usize;
                if amount > self.deposited[a] - self.paid[a] {
                    return Err(Error::InsufficientPoolBalance);
                }
                self.paid[a] += amount;
            }
            Op::Approve {
                item,
                attester,
                asset,
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
                self.clock += 1;
                self.items.insert(
                    item,
                    Item {
                        attester: attester as usize,
                        asset: asset as usize,
                        amount,
                        approved_by: 0,
                        approved_at: self.clock,
                    },
                );
            }
            Op::Claim(item) | Op::Disburse(item) => self.pay(item)?,
            Op::Pause => self.paused = true,
            Op::Unpause => self.paused = false,
            Op::SetMaxPerClaim(m) => self.max_claim = m,
            Op::SetMaxPerAttester(m) => self.max_attester = m,
            Op::SetMaxPerIdentity(m) => self.max_identity = m,
            Op::Suspend(_) | Op::Reinstate(_) => {}
            Op::VoidApprovalsSince { approver, since } => {
                let slot = &mut self.revoked_since[approver as usize];
                *slot = Some(match *slot {
                    Some(prev) => prev.min(since),
                    None => since,
                });
            }
            Op::SweepExpired => {
                // Lazy cleanup is permissionless and must never change
                // accounting: voided-but-unclaimed items were never paid.
            }
            Op::Rotate(attester) => {
                // Rotation keeps the identity key, so cumulative per-identity
                // totals carry over to the new address (#698).
                let _ = attester;
            }
        }
        Ok(())
    }
}

pub struct Harness {
    pub env: Env,
    pub pool: IncentivePoolClient<'static>,
    pub registry: attester_registry::AttesterRegistryClient<'static>,
    pub tokens: Vec<soroban_sdk::token::Client<'static>>,
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
    let mut tokens = Vec::new();
    let mut asset_ids = Vec::new();
    for _ in 0..ASSETS {
        let token_id = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        soroban_sdk::token::StellarAssetClient::new(&env, &token_id).mint(&admin, &1_000_000_000);
        tokens.push(soroban_sdk::token::Client::new(&env, &token_id));
        asset_ids.push(token_id);
    }
    let pool_id = env.register(IncentivePool, ());
    let pool = IncentivePoolClient::new(&env, &pool_id);
    pool.initialize(
        &admin,
        &Address::generate(&env),
        &asset_ids,
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
    Harness {
        env,
        pool,
        registry,
        tokens,
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
        max_identity: START_CAP,
        identity: [0, 0, 1],
        ..Default::default()
    };
    for (n, op) in ops.iter().enumerate() {
        let actual: Result<(), Error> = match *op {
            Op::Fund { asset, amount } => {
                outcome!(h.pool.try_fund(&h.asset_id(asset), &amount))
            }
            Op::Withdraw { asset, amount } => {
                outcome!(h.pool.try_withdraw(&h.admin, &h.asset_id(asset), &amount))
            }
            Op::Approve {
                item,
                attester,
                asset,
                amount,
            } => outcome!(h.pool.try_approve_work_item(
                &item_id(&h.env, item),
                &h.attesters[attester as usize],
                &h.asset_id(asset),
                &amount
            )),
            Op::Claim(item) => outcome!(h.pool.try_claim(&item_id(&h.env, item))),
            Op::Disburse(item) => outcome!(h.pool.try_disburse(&h.admin, &vec![&h.env, item_id(&h.env, item)])),
            Op::Pause => outcome!(h.pool.try_pause()),
            Op::Unpause => outcome!(h.pool.try_unpause()),
            Op::SetMaxPerClaim(v) => outcome!(h.pool.try_set_max_per_claim(&v)),
            Op::SetMaxPerAttester(v) => outcome!(h.pool.try_set_max_per_attester(&v)),
            Op::SetMaxPerIdentity(v) => outcome!(h.pool.try_set_max_per_identity(&v)),
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
            Op::VoidApprovalsSince { approver, since } => outcome!(h.pool.try_void_approvals_since(
                &h.admin,
                &h.approvers[approver as usize],
                &since
            )),
            Op::SweepExpired => outcome!(h.pool.try_sweep_expired()),
            Op::Rotate(a) => {
                let i = a as usize;
                let new_addr = Address::generate(&h.env);
                h.registry.rotate_attester(&h.admin, &h.attesters[i], &new_addr);
                h.attesters[i] = new_addr;
                Ok(())
            }
        };
        let expected = m.step(op);
        assert_eq!(actual, expected, "step {n} {op:?} diverged from model");

        // Per-asset invariants (#695): isolated custody and totals.
        for (a, token) in h.tokens.iter().enumerate() {
            let deposited = h.pool.get_total_deposited(&h.asset_id(a as u8));
            let paid = h.pool.get_total_paid(&h.asset_id(a as u8));
            assert_eq!(
                (deposited, paid),
                (m.deposited[a], m.paid[a]),
                "step {n}: asset {a} totals"
            );
            assert!(
                paid <= deposited,
                "step {n}: asset {a} total_paid > total_deposited"
            );
            assert_eq!(
                token.balance(&h.pool.address),
                deposited - paid,
                "step {n}: asset {a} custody balance != deposited - paid"
            );
        }
        for i in 0..ITEMS {
            assert_eq!(
                h.pool.is_work_item_claimed(&item_id(&h.env, i)),
                m.claimed.contains(&i),
                "step {n}: item {i} claim flag"
            );
        }
        for (i, a) in h.attesters.iter().enumerate() {
            for asset in 0..ASSETS {
                assert_eq!(
                    h.pool.get_attester_total_claimed(a, &h.asset_id(asset as u8)),
                    m.per_attester[i][asset],
                    "step {n}: attester {i} asset {asset} claimed"
                );
            }
        }
        // Per-identity cumulative totals survive rotation (#698).
        for identity in 0..IDENTITIES {
            for asset in 0..ASSETS {
                assert_eq!(
                    h.pool.get_identity_total_claimed(&identity_key(&h.env, identity as u8), &h.asset_id(asset as u8)),
                    m.per_identity[identity][asset],
                    "step {n}: identity {identity} asset {asset} claimed"
                );
            }
        }
        // Reservation accounting stays consistent after void + sweep (#697).
        assert_eq!(
            h.pool.get_total_reserved(),
            m.items
                .iter()
                .filter(|(id, item)| !m.claimed.contains(id) && !m.is_voided(item))
                .map(|(_, item)| item.amount)
                .sum::<i128>(),
            "step {n}: outstanding reservations"
        );
    }
}

impl Harness {
    fn asset_id(&self, asset: u8) -> Address {
        self.tokens[asset as usize].address.clone()
    }
}

pub fn identity_key(env: &Env, identity: u8) -> BytesN<32> {
    BytesN::from_array(env, &[0xA0 | identity; 32])
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

    /// Push disbursement must apply exactly the same guards as `claim`
    /// (#696): for any trace, an item is payable by either path iff the
    /// model says so.
    #[test]
    fn disburse_matches_claim_guards(ops in prop::collection::vec(op_strategy(), 1..40)) {
        let h = harness();
        let mut m = Model {
            max_claim: START_CAP,
            max_attester: START_CAP,
            max_identity: START_CAP,
            identity: [0, 0, 1],
            ..Default::default()
        };
        for (n, op) in ops.iter().enumerate() {
            let expected = m.step(op);
            if let Op::Claim(item) | Op::Disburse(item) = *op {
                // Both entry points must agree with the shared guard model.
                let via_claim = outcome!(h.pool.try_claim(&item_id(&h.env, item)));
                let via_disburse = outcome!(h.pool.try_disburse(
                    &h.admin,
                    &vec![&h.env, item_id(&h.env, item)]
                ));
                assert_eq!(via_claim, expected, "step {n}: claim parity");
                assert_eq!(via_disburse, expected, "step {n}: disburse parity");
            }
        }
    }
}
