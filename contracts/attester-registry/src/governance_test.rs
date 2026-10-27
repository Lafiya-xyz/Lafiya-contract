extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{token, Env, Map};

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
