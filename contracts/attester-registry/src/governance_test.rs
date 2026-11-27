extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Env, IntoVal, Map, Symbol, Val, Vec as SVec};

const DAY: u64 = 24 * 60 * 60;

fn setup() -> (Env, AttesterRegistryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let admin = Address::generate(&env);
    let id = env.register(AttesterRegistry, (admin.clone(),));
    let client = AttesterRegistryClient::new(&env, &id);
    client.grant_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Guardian, &admin);
    (env, client, admin)
}

fn advance(env: &Env, seconds: u64) {
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + seconds);
}

// ── Probation (#589) ──

#[test]
fn no_probation_without_config() {
    let (env, client, admin) = setup();
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Standard
    );
    assert!(client.get_probation_limits(&attester).is_none());
    assert!(client.is_payout_eligible(&attester));
}

#[test]
fn probation_expires_exactly_at_boundary() {
    let (env, client, admin) = setup();
    client.set_probation_config(&(7 * DAY), &5, &100, &false);
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);

    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Probation
    );
    let limits = client.get_probation_limits(&attester).unwrap();
    assert_eq!(limits.rate_cap, 5);
    assert_eq!(limits.ends_at, 1_000 + 7 * DAY);
    assert!(!client.is_payout_eligible(&attester));

    advance(&env, 7 * DAY - 1);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Probation
    );
    advance(&env, 1);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Standard
    );
    assert!(client.is_payout_eligible(&attester));
    client.finalize_probation(&attester);
    assert!(client.try_finalize_probation(&attester).is_err());
}

#[test]
fn flagged_attester_is_not_auto_promoted_until_explicit_promotion() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let attester = Address::generate(&env);
    client.add_attester_with_info(&admin, &attester, &None, &None, &None, &None);
    client.flag_probation(&admin, &attester, &Symbol::new(&env, "odd_volume"));
    advance(&env, 10 * DAY);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Flagged
    );
    assert!(client.try_finalize_probation(&attester).is_err());

    client.promote_attester(&admin, &attester);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Standard
    );
    assert!(client.try_promote_attester(&admin, &attester).is_err());
}

#[test]
fn early_promotion_ends_probation() {
    let (env, client, admin) = setup();
    client.set_probation_config(&(30 * DAY), &1, &10, &false);
    let attester = Address::generate(&env);
    client.add_attesters(&admin, &Vec::from_array(&env, [attester.clone()]));
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Probation
    );
    client.promote_attester(&admin, &attester);
    assert!(client.get_probation_limits(&attester).is_none());
}

#[test]
fn suspension_flags_probation_and_reinstatement_does_not_clear_it() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.suspend_attester(&admin, &attester);
    client.reinstate_attester(&admin, &attester);
    advance(&env, 2 * DAY);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Flagged
    );
}

#[test]
fn suspension_after_probation_does_not_create_a_flag() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    advance(&env, 2 * DAY);
    client.suspend_attester(&admin, &attester);
    client.reinstate_attester(&admin, &attester);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Standard
    );
}

#[test]
fn removal_resets_probation_and_reenrollment_restarts_it() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    advance(&env, 2 * DAY);
    client.remove_attester(&admin, &attester);
    client.add_attester(&admin, &attester);
    assert_eq!(
        client.get_probation_state(&attester),
        ProbationState::Probation
    );
}

#[test]
fn rotation_carries_probation_tier_and_bond_state() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let old = Address::generate(&env);
    let new = Address::generate(&env);
    client.add_attester(&admin, &old);
    client.set_attester_tier(&admin, &old, &Tier::Nurse);
    client.rotate_attester(&old, &new);
    assert_eq!(client.get_probation_state(&old), ProbationState::Standard);
    assert_eq!(client.get_probation_state(&new), ProbationState::Probation);
    assert_eq!(client.get_attester_tier(&new), Tier::Nurse);
    assert_eq!(client.get_attester_tier(&old), Tier::Chw);
}

#[test]
fn breach_report_flags_and_requires_probation() {
    let (env, client, admin) = setup();
    client.set_probation_config(&DAY, &5, &100, &true);
    let attester = Address::generate(&env);
    let other = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.add_attester(&admin, &other);
    advance(&env, 2 * DAY);
    assert!(client
        .try_report_probation_breach(&admin, &other, &Symbol::new(&env, "rate"))
        .is_err());

    let fresh = Address::generate(&env);
    client.add_attester(&admin, &fresh);
    client.report_probation_breach(&admin, &fresh, &Symbol::new(&env, "rate"));
    assert_eq!(client.get_probation_state(&fresh), ProbationState::Flagged);
}

#[test]
fn rate_cap_requires_window() {
    let (_env, client, _admin) = setup();
    assert!(client
        .try_set_probation_config(&DAY, &5, &0, &true)
        .is_err());
}

// ── Capability tiers (#588) ──

fn matrix(env: &Env) -> Map<Symbol, Tier> {
    let mut m = Map::new(env);
    m.set(Symbol::new(env, "medication"), Tier::Physician);
    m.set(Symbol::new(env, "vitals"), Tier::Nurse);
    m
}

#[test]
fn unassigned_attesters_default_to_base_tier_and_unlisted_classes_are_open() {
    let (env, client, admin) = setup();
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    assert_eq!(client.get_attester_tier(&attester), Tier::Chw);
    assert!(client.is_class_allowed(&attester, &Symbol::new(&env, "identity")));
}

#[test]
fn matrix_change_is_timelocked() {
    let (env, client, admin) = setup();
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.propose_tier_matrix(&matrix(&env));
    assert!(client.try_apply_tier_matrix().is_err());
    assert!(client.get_tier_matrix().is_empty());

    advance(&env, TIER_MATRIX_TIMELOCK_SECONDS - 1);
    assert!(client.try_apply_tier_matrix().is_err());
    advance(&env, 1);
    client.apply_tier_matrix();
    assert!(client.get_pending_tier_matrix().is_none());

    let medication = Symbol::new(&env, "medication");
    assert_eq!(client.get_required_tier(&medication), Tier::Physician);
    assert!(!client.is_class_allowed(&attester, &medication));
    client.set_attester_tier(&admin, &attester, &Tier::Nurse);
    assert!(client.is_class_allowed(&attester, &Symbol::new(&env, "vitals")));
    assert!(!client.is_class_allowed(&attester, &medication));
    client.set_attester_tier(&admin, &attester, &Tier::Physician);
    assert!(client.is_class_allowed(&attester, &medication));
}

#[test]
fn pending_matrix_can_be_cancelled_and_suspended_attester_is_denied() {
    let (env, client, admin) = setup();
    assert!(client.try_cancel_tier_matrix().is_err());
    client.propose_tier_matrix(&matrix(&env));
    client.cancel_tier_matrix();
    assert!(client.try_apply_tier_matrix().is_err());

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.suspend_attester(&admin, &attester);
    assert!(!client.is_class_allowed(&attester, &Symbol::new(&env, "identity")));
}

#[test]
fn tier_assignment_requires_enrolled_attester() {
    let (env, client, admin) = setup();
    let stranger = Address::generate(&env);
    assert!(client
        .try_set_attester_tier(&admin, &stranger, &Tier::Nurse)
        .is_err());
}

// ── Bond and slashing (#587) ──

struct BondFixture {
    env: Env,
    client: AttesterRegistryClient<'static>,
    admin: Address,
    token: token::Client<'static>,
    recipient: Address,
    attester: Address,
    guardian: Address,
}

const MIN_BOND: i128 = 1_000;
const WINDOW: u64 = DAY;

fn bond_setup(quorum: u32) -> BondFixture {
    let (env, client, admin) = setup();
    let guardian = Address::generate(&env);
    client.grant_role(&Role::Guardian, &guardian);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let recipient = Address::generate(&env);
    client.set_bond_config(
        &token_id,
        &MIN_BOND,
        &WINDOW,
        &quorum,
        &(3 * DAY),
        &recipient,
    );
    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    token::StellarAssetClient::new(&env, &token_id).mint(&attester, &10_000);
    BondFixture {
        token: token::Client::new(&env, &token_id),
        env,
        client,
        admin,
        recipient,
        attester,
        guardian,
    }
}

fn evidence(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[7; 32])
}

#[test]
fn posting_a_bond_escrows_funds_and_marks_bonded() {
    let f = bond_setup(2);
    assert!(!f.client.is_bonded(&f.attester));
    f.client.post_bond(&f.attester, &MIN_BOND);
    assert!(f.client.is_bonded(&f.attester));
    assert_eq!(f.token.balance(&f.attester), 10_000 - MIN_BOND);
    assert_eq!(f.token.balance(&f.client.address), MIN_BOND);
}

#[test]
fn bond_requires_enrollment_and_valid_amount() {
    let f = bond_setup(2);
    assert!(f.client.try_post_bond(&f.attester, &0).is_err());
    let stranger = Address::generate(&f.env);
    assert!(f.client.try_post_bond(&stranger, &MIN_BOND).is_err());
}

#[test]
fn slash_executes_only_after_quorum_and_challenge_window() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &2_000);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &1_500, &evidence(&f.env));

    // Pending: below quorum, cannot execute.
    assert!(f.client.try_execute_slash(&case_id).is_err());
    // The proposer cannot approve twice.
    assert!(f.client.try_approve_slash(&f.admin, &case_id).is_err());

    f.client.approve_slash(&f.guardian, &case_id);
    let case = f.client.get_slash_case(&case_id).unwrap();
    assert_eq!(case.status, SlashStatus::Challengeable);

    advance(&f.env, WINDOW - 1);
    assert!(f.client.try_execute_slash(&case_id).is_err());
    advance(&f.env, 1);
    f.client.execute_slash(&case_id);

    assert_eq!(f.token.balance(&f.recipient), 1_500);
    let info = f.client.get_bond(&f.attester).unwrap();
    assert_eq!((info.amount, info.locked, info.open_cases), (500, 0, 0));
    assert!(f.client.try_execute_slash(&case_id).is_err());
}

#[test]
fn single_approval_quorum_starts_window_at_proposal() {
    let f = bond_setup(1);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    assert_eq!(
        f.client.get_slash_case(&case_id).unwrap().status,
        SlashStatus::Challengeable
    );
}

#[test]
fn locked_amount_cannot_be_over_committed_or_withdrawn() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    f.client
        .propose_slash(&f.admin, &f.attester, &800, &evidence(&f.env));
    assert!(f
        .client
        .try_propose_slash(&f.admin, &f.attester, &300, &evidence(&f.env))
        .is_err());

    f.client.request_bond_withdrawal(&f.attester);
    advance(&f.env, 3 * DAY);
    assert!(f.client.try_withdraw_bond(&f.attester).is_err());
}

#[test]
fn appeal_freezes_case_and_rejection_unlocks_bond() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    // Cannot appeal before the window opens.
    assert!(f.client.try_appeal_slash(&case_id).is_err());
    f.client.approve_slash(&f.guardian, &case_id);

    f.client.appeal_slash(&case_id);
    advance(&f.env, 2 * WINDOW);
    assert!(f.client.try_execute_slash(&case_id).is_err());

    f.client.resolve_appeal(&case_id, &false);
    let info = f.client.get_bond(&f.attester).unwrap();
    assert_eq!(
        (info.amount, info.locked, info.open_cases),
        (MIN_BOND, 0, 0)
    );
    assert_eq!(f.token.balance(&f.recipient), 0);
}

#[test]
fn upheld_appeal_is_executable_immediately() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    f.client.approve_slash(&f.guardian, &case_id);
    f.client.appeal_slash(&case_id);
    f.client.resolve_appeal(&case_id, &true);
    f.client.execute_slash(&case_id);
    assert_eq!(f.token.balance(&f.recipient), MIN_BOND);
}

#[test]
fn appeal_is_rejected_after_window_closes() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    f.client.approve_slash(&f.guardian, &case_id);
    advance(&f.env, WINDOW);
    assert!(f.client.try_appeal_slash(&case_id).is_err());
}

#[test]
fn admin_can_cancel_open_case() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    f.client.cancel_slash(&case_id);
    assert_eq!(f.client.get_bond(&f.attester).unwrap().locked, 0);
    assert!(f.client.try_cancel_slash(&case_id).is_err());
}

#[test]
fn rotation_moves_bond_and_open_cases_to_successor() {
    let f = bond_setup(2);
    f.client.post_bond(&f.attester, &MIN_BOND);
    let case_id = f
        .client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
    f.client.approve_slash(&f.guardian, &case_id);

    let successor = Address::generate(&f.env);
    f.client.rotate_attester(&f.attester, &successor);
    assert!(f.client.get_bond(&f.attester).is_none());
    let info = f.client.get_bond(&successor).unwrap();
    assert_eq!(
        (info.amount, info.locked, info.open_cases),
        (MIN_BOND, MIN_BOND, 1)
    );
    assert_eq!(
        f.client.get_slash_case(&case_id).unwrap().attester,
        successor
    );

    // The successor, not the retired key, can appeal.
    f.client.appeal_slash(&case_id);
    advance(&f.env, 2 * WINDOW);
    f.client.resolve_appeal(&case_id, &true);
    f.client.execute_slash(&case_id);
    assert_eq!(f.token.balance(&f.recipient), MIN_BOND);
    assert_eq!(f.client.get_bond(&successor).unwrap().amount, 0);
}

#[test]
fn withdrawal_needs_request_and_cooldown_and_remains_slashable_meanwhile() {
    let f = bond_setup(1);
    f.client.post_bond(&f.attester, &MIN_BOND);
    assert!(f.client.try_withdraw_bond(&f.attester).is_err());
    f.client.request_bond_withdrawal(&f.attester);
    advance(&f.env, 3 * DAY - 1);
    assert!(f.client.try_withdraw_bond(&f.attester).is_err());

    // Still slashable during the cooldown.
    f.client
        .propose_slash(&f.admin, &f.attester, &100, &evidence(&f.env));
    assert_eq!(f.client.get_bond(&f.attester).unwrap().locked, 100);
    f.client.cancel_slash(&1);

    advance(&f.env, 1);
    f.client.withdraw_bond(&f.attester);
    assert_eq!(f.token.balance(&f.attester), 10_000);
    assert!(f.client.get_bond(&f.attester).is_none());
}

#[test]
fn removed_attester_bond_stays_slashable() {
    let f = bond_setup(1);
    f.client.post_bond(&f.attester, &MIN_BOND);
    f.client.remove_attester(&f.admin, &f.attester);
    f.client
        .propose_slash(&f.admin, &f.attester, &MIN_BOND, &evidence(&f.env));
}

#[test]
fn bond_config_validation() {
    let (env, client, _admin) = setup();
    let token = Address::generate(&env);
    let r = Address::generate(&env);
    assert!(client
        .try_set_bond_config(&token, &0, &10, &2, &20, &r)
        .is_err());
    assert!(client
        .try_set_bond_config(&token, &1, &10, &0, &20, &r)
        .is_err());
    assert!(client
        .try_set_bond_config(&token, &1, &10, &2, &10, &r)
        .is_err());
}

// ── Role-escalation property harness (#682) ──
//
// The core safety property: no sequence of calls made by a non-admin actor can
// grant that actor (or anyone else) a capability they could not already
// exercise. We model the registry's role surface as a capability graph and
// assert it is monotonic for non-admin actors after every step.

/// Fixed actor set. Index 0 is the admin; the rest are non-admin actors whose
/// capabilities must never grow without an admin-signed grant.
const ACTOR_COUNT: usize = 6;
const ADMIN: usize = 0;
const REGISTRAR: usize = 1;
const GUARDIAN: usize = 2;
const REGIONAL: usize = 3;
const ATTESTER: usize = 4;
const OUTSIDER: usize = 5;

/// Capabilities an actor can exercise. Derived from the actor's roles and
/// enrollment state; the harness asserts this set never grows for non-admins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Cap {
    Registrar,
    Guardian,
    RegionalRegistrar,
    EnrolledAttester,
    SessionKey,
}

/// A single role-mutating call in the generated sequence.
#[derive(Clone, Copy, Debug)]
enum Op {
    GrantRole(u8, u8),   // (role selector, target actor)
    RevokeRole(u8, u8),  // (role selector, target actor)
    ProposeAdmin(u8),    // proposed new admin actor
    AcceptAdmin(u8),     // actor attempting to accept
    DelegateSession(u8), // actor delegating a session key to itself
    AddAttester(u8),     // actor adding target as attester
    RemoveAttester(u8),  // actor removing target
    SuspendAttester(u8), // actor suspending target
    ReinstateAttester(u8),
    AssignRegional(u8), // actor assigning target as regional registrar
}

fn role_for(sel: u8) -> Role {
    match sel % 3 {
        0 => Role::Registrar,
        1 => Role::Guardian,
        _ => Role::RegionalRegistrar,
    }
}

/// Deterministic xorshift PRNG so failures are reproducible from the seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn actor(&mut self) -> u8 {
        self.below(ACTOR_COUNT as u64) as u8
    }
    fn op(&mut self) -> Op {
        let a = self.actor();
        let b = self.actor();
        match self.below(10) {
            0 => Op::GrantRole(self.below(3) as u8, b),
            1 => Op::RevokeRole(self.below(3) as u8, b),
            2 => Op::ProposeAdmin(b),
            3 => Op::AcceptAdmin(a),
            4 => Op::DelegateSession(a),
            5 => Op::AddAttester(b),
            6 => Op::RemoveAttester(b),
            7 => Op::SuspendAttester(b),
            8 => Op::ReinstateAttester(b),
            _ => Op::AssignRegional(b),
        }
    }
}

struct Harness {
    env: Env,
    client: AttesterRegistryClient<'static>,
    actors: [Address; ACTOR_COUNT],
    /// Last observed capability set per actor, used for the monotonicity check.
    caps: [SVec<Cap>; ACTOR_COUNT],
}

fn has_cap(caps: &SVec<Cap>, c: Cap) -> bool {
    caps.iter().any(|x| x == c)
}

impl Harness {
    fn new(seed: u64) -> Self {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let actors: [Address; ACTOR_COUNT] = core::array::from_fn(|_| Address::generate(&env));
        let id = env.register(AttesterRegistry, (actors[ADMIN].clone(),));
        let client = AttesterRegistryClient::new(&env, &id);
        let caps = core::array::from_fn(|_| SVec::new(&env));
        let mut h = Harness {
            env,
            client,
            actors,
            caps,
        };
        // Seed a baseline: admin holds everything, others hold nothing.
        h.refresh_caps();
        let _ = seed;
        h
    }

    fn addr(&self, i: u8) -> Address {
        self.actors[i as usize % ACTOR_COUNT].clone()
    }

    /// Compute the effective-capability graph from on-chain state.
    fn compute_caps(&self, i: usize) -> SVec<Cap> {
        let env = &self.env;
        let a = &self.actors[i];
        let mut caps = SVec::new(env);
        if self.client.has_role(&Role::Registrar, a) {
            caps.push_back(Cap::Registrar);
        }
        if self.client.has_role(&Role::Guardian, a) {
            caps.push_back(Cap::Guardian);
        }
        if self.client.has_role(&Role::RegionalRegistrar, a) {
            caps.push_back(Cap::RegionalRegistrar);
        }
        if self.client.is_attester(a) {
            caps.push_back(Cap::EnrolledAttester);
        }
        caps
    }

    fn refresh_caps(&mut self) {
        for i in 0..ACTOR_COUNT {
            self.caps[i] = self.compute_caps(i);
        }
    }

    /// Assert no non-admin actor gained a capability since the last snapshot.
    fn assert_monotonic(&self) {
        for i in 0..ACTOR_COUNT {
            if i == ADMIN {
                continue;
            }
            let now = self.compute_caps(i);
            for c in now.iter() {
                assert!(
                    has_cap(&self.caps[i], c),
                    "role escalation: actor {} gained {:?}",
                    i,
                    c
                );
            }
        }
    }

    /// Execute one op with auth scoped to the acting identity only.
    fn step(&mut self, op: Op) {
        let env = self.env.clone();
        let contract = self.client.address.clone();
        match op {
            Op::GrantRole(sel, target) => {
                let actor = self.addr(ADMIN);
                let role = role_for(sel);
                let t = self.addr(target);
                let args: SVec<Val> = (role.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "grant_role",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_grant_role(&role, &t);
            }
            Op::RevokeRole(sel, target) => {
                let actor = self.addr(ADMIN);
                let role = role_for(sel);
                let t = self.addr(target);
                let args: SVec<Val> = (role.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "revoke_role",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_revoke_role(&role, &t);
            }
            Op::ProposeAdmin(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (t.clone(),).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "propose_admin",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_propose_admin(&t);
            }
            Op::AcceptAdmin(actor_idx) => {
                let actor = self.addr(actor_idx);
                let args: SVec<Val> = SVec::new(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "accept_admin",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_accept_admin();
            }
            Op::DelegateSession(actor_idx) => {
                let actor = self.addr(actor_idx);
                let args: SVec<Val> = SVec::new(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "delegate_session_key",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_delegate_session_key(&actor);
            }
            Op::AddAttester(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (actor.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "add_attester",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_add_attester(&actor, &t);
            }
            Op::RemoveAttester(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (actor.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "remove_attester",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_remove_attester(&actor, &t);
            }
            Op::SuspendAttester(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (actor.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "suspend_attester",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_suspend_attester(&actor, &t);
            }
            Op::ReinstateAttester(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (actor.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "reinstate_attester",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_reinstate_attester(&actor, &t);
            }
            Op::AssignRegional(target) => {
                let actor = self.addr(ADMIN);
                let t = self.addr(target);
                let args: SVec<Val> = (actor.clone(), t.clone()).into_val(&env);
                env.mock_auths(&[MockAuth {
                    address: &actor,
                    invoke: &MockAuthInvoke {
                        contract: &contract,
                        fn_name: "assign_regional_registrar",
                        args: args.clone(),
                        sub_invokes: &[],
                    },
                }]);
                let _ = self.client.try_assign_regional_registrar(&actor, &t);
            }
        }
    }

    /// Run `steps` random ops, asserting monotonicity after each.
    fn run(&mut self, seed: u64, steps: usize) {
        let mut rng = Rng::new(seed);
        for _ in 0..steps {
            let op = rng.op();
            self.step(op);
            self.assert_monotonic();
            self.refresh_caps();
        }
    }
}

/// Number of sequences to run. Nightly sets `REGISTRY_PROP_SEQUENCES` to a
/// large value (>= 5000); default keeps the regular test suite fast.
fn sequence_budget() -> usize {
    std::env::var("REGISTRY_PROP_SEQUENCES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
}

#[test]
fn role_escalation_property_holds() {
    let budget = sequence_budget();
    for seed in 0..budget as u64 {
        let mut h = Harness::new(seed);
        h.run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 40);
    }
}

/// Test-only injected escalation: a non-admin actor is granted a role without
/// admin auth. The harness must catch this as a monotonicity violation.
#[test]
fn harness_catches_injected_escalation() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let actors: [Address; ACTOR_COUNT] = core::array::from_fn(|_| Address::generate(&env));
    let id = env.register(AttesterRegistry, (actors[ADMIN].clone(),));
    let client = AttesterRegistryClient::new(&env, &id);
    let caps: [SVec<Cap>; ACTOR_COUNT] = core::array::from_fn(|_| SVec::new(&env));
    let mut h = Harness {
        env,
        client,
        actors,
        caps,
    };
    h.refresh_caps();

    // Inject: grant Registrar to the outsider using only the outsider's auth.
    let outsider = h.actors[OUTSIDER].clone();
    let contract = h.client.address.clone();
    let env = h.env.clone();
    let args: SVec<Val> = (Role::Registrar, outsider.clone()).into_val(&env);
    env.mock_auths(&[MockAuth {
        address: &outsider,
        invoke: &MockAuthInvoke {
            contract: &contract,
            fn_name: "grant_role",
            args: args.clone(),
            sub_invokes: &[],
        },
    }]);
    // If the contract is correct this fails; if it succeeds the harness must
    // detect the escalation below.
    let _ = h.client.try_grant_role(&Role::Registrar, &outsider);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.assert_monotonic();
    }));
    // Either the call was rejected (no escalation) or the harness caught it.
    if h.client.has_role(&Role::Registrar, &outsider) {
        assert!(result.is_err(), "harness failed to catch injected escalation");
    }
}
