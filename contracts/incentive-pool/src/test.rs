extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, BytesN, Env, Event, IntoVal};

const ONE_UNIT: i128 = 1_000_000; // 1 USDC (6 decimals)
const POOL_CAP: i128 = 100_000 * ONE_UNIT; // 100 000 USDC
const CLAIM_CAP: i128 = 50 * ONE_UNIT; // 50 USDC per claim

fn setup() -> (
    Env,
    IncentivePoolClient<'static>,
    attester_registry::AttesterRegistryClient<'static>,
    Address,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let admin = Address::generate(&env);
    attester_registry_client.initialize(&admin);

    // Deploy a token (Stellar asset contract for testing).
    let token_admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract(token_admin.clone());

    let approver = Address::generate(&env);

    let contract_id = env.register(IncentivePool, ());
    let client = IncentivePoolClient::new(&env, &contract_id);

    client.initialize(
        &admin,
        &approver,
        &token,
        &attester_registry_id,
        &CLAIM_CAP,
        &POOL_CAP,
    );

    (
        env,
        client,
        attester_registry_client,
        admin,
        approver,
        token,
        token_admin,
    )
}

// ─────────────────────── Initialization ───────────────────────

#[test]
fn initialize_succeeds() {
    let (env, client, attester_registry, admin, approver, token, _token_admin) = setup();

    assert_eq!(client.get_admin(), Ok(admin));
    assert_eq!(client.get_approver(), Ok(approver));
    assert_eq!(client.get_attester_registry(), Ok(attester_registry.address));
    assert_eq!(client.get_token(), Ok(token));
    assert_eq!(client.get_max_per_claim(), Ok(CLAIM_CAP));
    assert_eq!(client.get_max_per_attester(), Ok(POOL_CAP));
    assert_eq!(client.get_total_deposited(), Ok(0));
    assert_eq!(client.get_total_paid(), Ok(0));
    assert!(!client.is_paused());

    // Initialized event should be emitted.
    let events = env.events().all();
    assert!(!events.is_empty());
}

#[test]
fn double_initialize_fails() {
    let (_env, client, _registry, admin, _approver, _token, _ta) = setup();

    let env = Env::default();
    env.mock_all_auths();
    let result = client.try_initialize(
        &admin,
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        &CLAIM_CAP,
        &POOL_CAP,
    );
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn initialize_rejects_invalid_attester_registry() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(IncentivePool, ());
    let client = IncentivePoolClient::new(&env, &contract_id);
    let token_admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract(token_admin);
    let non_contract = Address::generate(&env);

    let result = client.try_initialize(
        &admin,
        &Address::generate(&env),
        &token,
        &non_contract,
        &CLAIM_CAP,
        &POOL_CAP,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRegistryWiring)));
}

// ─────────────────────── Funding ───────────────────────

#[test]
fn fund_succeeds() {
    let (env, client, _registry, admin, _approver, _token, _token_admin) = setup();

    let amount = 1000 * ONE_UNIT;
    client.fund(&amount);

    assert_eq!(client.get_total_deposited(), Ok(amount));

    let expected_event = PoolFunded {
        funder: admin,
        amount,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn fund_by_non_admin_fails() {
    let (env, client, _registry, _admin, _approver, _token, _token_admin) = setup();
    let non_admin = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "fund",
            args: (ONE_UNIT,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_fund(&ONE_UNIT);
    assert!(result.is_err());
    assert_eq!(client.get_total_deposited(), Ok(0));
}

#[test]
fn fund_zero_amount_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _token_admin) = setup();

    let result = client.try_fund(&0);
    assert_eq!(result, Err(Ok(Error::NonPositiveAmount)));
}

#[test]
fn fund_negative_amount_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _token_admin) = setup();

    let result = client.try_fund(&(-ONE_UNIT));
    assert_eq!(result, Err(Ok(Error::NonPositiveAmount)));
}

#[test]
fn fund_accumulates_deposited_total() {
    let (_env, client, _registry, _admin, _approver, _token, _token_admin) = setup();

    client.fund(&ONE_UNIT);
    client.fund(&(2 * ONE_UNIT));
    assert_eq!(client.get_total_deposited(), Ok(3 * ONE_UNIT));
}

// ─────────────────────── Withdraw ───────────────────────

#[test]
fn withdraw_succeeds_after_funding() {
    let (env, client, _registry, admin, _approver, _token, _token_admin) = setup();

    client.fund(&ONE_UNIT);
    let recipient = Address::generate(&env);
    client.withdraw(&recipient, &ONE_UNIT);

    // Total deposited unchanged, total paid updated.
    assert_eq!(client.get_total_deposited(), Ok(ONE_UNIT));
    assert_eq!(client.get_total_paid(), Ok(ONE_UNIT));

    let expected_event = PoolWithdrawn {
        to: recipient,
        amount: ONE_UNIT,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn withdraw_exceeding_balance_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _token_admin) = setup();

    client.fund(&ONE_UNIT);
    let recipient = Address::generate(&env);
    let result = client.try_withdraw(&recipient, &(2 * ONE_UNIT));
    assert_eq!(result, Err(Ok(Error::InsufficientPoolBalance)));
}

#[test]
fn withdraw_zero_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _token_admin) = setup();
    client.fund(&ONE_UNIT);
    let recipient = Address::generate(&env);
    let result = client.try_withdraw(&recipient, &0);
    assert_eq!(result, Err(Ok(Error::NonPositiveAmount)));
}

#[test]
fn withdraw_by_non_admin_fails() {
    let (env, client, _registry, _admin, _approver, _token, _token_admin) = setup();
    client.fund(&ONE_UNIT);
    let non_admin = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "withdraw",
            args: (recipient.clone(), ONE_UNIT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_withdraw(&recipient, &ONE_UNIT);
    assert!(result.is_err());
}

// ─────────────────────── Approve work item ───────────────────────

#[test]
fn approve_work_item_succeeds() {
    let (env, client, attester_registry, _admin, approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    assert!(client.is_work_item_approved(work_item_id.clone()));
    assert!(!client.is_work_item_claimed(work_item_id.clone()));

    let item = client.get_work_item(&work_item_id).unwrap();
    assert_eq!(item.attester, attester);
    assert_eq!(item.payout_amount, ONE_UNIT);

    let expected_event = WorkItemApproved {
        work_item_id,
        attester: item.attester.clone(),
        payout_amount: ONE_UNIT,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn approve_work_item_by_non_approver_fails() {
    let (env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let non_approver = Address::generate(&env);
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_approver,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "approve_work_item",
            args: (work_item_id.clone(), attester.clone(), ONE_UNIT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_approve_work_item(&work_item_id, &attester, &ONE_UNIT);
    assert!(result.is_err());
    assert!(!client.is_work_item_approved(work_item_id));
}

#[test]
fn approve_work_item_duplicate_fails() {
    let (_env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    let result = client.try_approve_work_item(&work_item_id, &attester, &ONE_UNIT);
    assert_eq!(result, Err(Ok(Error::WorkItemAlreadyApproved)));
}

#[test]
fn approve_work_item_non_allowlisted_attester_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);

    let result = client.try_approve_work_item(&work_item_id, &attester, &ONE_UNIT);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
}

#[test]
fn approve_work_item_exceeding_per_claim_cap_fails() {
    let (_env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    let over_cap = CLAIM_CAP + 1;
    let result = client.try_approve_work_item(&work_item_id, &attester, &over_cap);
    assert_eq!(result, Err(Ok(Error::PayoutCapExceeded)));
}

#[test]
fn approve_work_item_zero_amount_fails() {
    let (_env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_approve_work_item(&work_item_id, &attester, &0);
    assert_eq!(result, Err(Ok(Error::NonPositiveAmount)));
}

// ─────────────────────── Claim ───────────────────────

#[test]
fn claim_succeeds() {
    let (env, client, attester_registry, _admin, _approver, token, _token_admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    // Fund the pool.
    client.fund(&ONE_UNIT);

    // Approve a work item.
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    // Claim the payout.
    client.claim(&work_item_id);

    assert!(client.is_work_item_claimed(work_item_id.clone()));
    assert_eq!(client.get_total_paid(), Ok(ONE_UNIT));
    assert_eq!(client.get_attester_total_claimed(attester.clone()), ONE_UNIT);

    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&attester), ONE_UNIT);

    let expected_event = PayoutClaimed {
        work_item_id,
        attester,
        amount: ONE_UNIT,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn claim_unapproved_work_item_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    client.fund(&ONE_UNIT);

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_claim(&work_item_id);
    assert_eq!(result, Err(Ok(Error::WorkItemNotApproved)));
}

#[test]
fn claim_already_claimed_fails() {
    let (_env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    client.fund(&ONE_UNIT);
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);
    client.claim(&work_item_id);

    let result = client.try_claim(&work_item_id);
    assert_eq!(result, Err(Ok(Error::WorkItemAlreadyClaimed)));
}

#[test]
fn claim_suspended_attester_fails() {
    let (_env, client, attester_registry, admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    client.fund(&ONE_UNIT);
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    // Suspend the attester after approval.
    attester_registry.suspend_attester(&attester);

    let result = client.try_claim(&work_item_id);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    let _ = admin;
}

#[test]
fn claim_exceeding_per_attester_cap_fails() {
    let (env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    // Fund enough for two claims.
    client.fund(&(2 * POOL_CAP));

    // Approve and claim first item (at the cap).
    let id1 = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&id1, &attester, &POOL_CAP);
    client.claim(&id1);

    // Approve second item — still at cap, but claiming would exceed it.
    let id2 = BytesN::from_array(&env, &[2u8; 32]);
    client.approve_work_item(&id2, &attester, &ONE_UNIT);
    let result = client.try_claim(&id2);
    assert_eq!(result, Err(Ok(Error::AttesterClaimCapExceeded)));
}

#[test]
fn claim_insufficient_pool_balance_fails() {
    let (_env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    // Fund only 1 unit.
    client.fund(&ONE_UNIT);

    // Approve a work item for the full amount.
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    // Withdraw most of the funds.
    let recipient = Address::generate(&env);
    client.withdraw(&recipient, &(ONE_UNIT - 1));

    // Now claim should fail because pool balance is insufficient.
    let result = client.try_claim(&work_item_id);
    assert_eq!(result, Err(Ok(Error::InsufficientPoolBalance)));
}

// ─────────────────────── Pause ───────────────────────

#[test]
fn pause_blocks_fund() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    client.pause();
    let result = client.try_fund(&ONE_UNIT);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_withdraw() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    client.fund(&ONE_UNIT);
    client.pause();
    let recipient = Address::generate(&env);
    let result = client.try_withdraw(&recipient, &ONE_UNIT);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_approve() {
    let (env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    client.pause();

    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_approve_work_item(&work_item_id, &attester, &ONE_UNIT);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_claim() {
    let (env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    client.fund(&ONE_UNIT);
    let work_item_id = BytesN::from_array(&env, &[1u8; 32]);
    client.approve_work_item(&work_item_id, &attester, &ONE_UNIT);

    client.pause();
    let result = client.try_claim(&work_item_id);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_unpause_cycle() {
    let (env, client, _registry, admin, _approver, _token, _ta) = setup();

    assert!(!client.is_paused());
    client.pause();
    assert!(client.is_paused());
    client.unpause();
    assert!(!client.is_paused());

    // Verify events
    let expected = std::vec![
        Paused { by: admin.clone() }.to_xdr(&env, &client.address),
        Unpaused { by: admin }.to_xdr(&env, &client.address),
    ];
    assert_eq!(env.events().all(), expected);
}

#[test]
fn pause_by_non_admin_fails() {
    let (env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let non_admin = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "pause",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_pause();
    assert!(result.is_err());
    assert!(!client.is_paused());
}

// ─────────────────────── Admin configuration ───────────────────────

#[test]
fn set_approver_succeeds() {
    let (env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let new_approver = Address::generate(&env);
    client.set_approver(&new_approver);
    assert_eq!(client.get_approver(), Ok(new_approver));
}

#[test]
fn set_max_per_claim_succeeds() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let new_cap = 10 * ONE_UNIT;
    client.set_max_per_claim(&new_cap);
    assert_eq!(client.get_max_per_claim(), Ok(new_cap));
}

#[test]
fn set_max_per_attester_succeeds() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let new_cap = 1_000 * ONE_UNIT;
    client.set_max_per_attester(&new_cap);
    assert_eq!(client.get_max_per_attester(), Ok(new_cap));
}

#[test]
fn set_attester_registry_succeeds() {
    let (env, client, _registry, admin, _approver, _token, _ta) = setup();
    let new_registry = Address::generate(&env);

    client.set_attester_registry(&new_registry);
    assert_eq!(client.get_attester_registry(), Ok(new_registry));

    let expected_event = AttesterRegistryRepointed {
        previous: _registry.address,
        new: new_registry.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
    let _ = admin;
}

// ─────────────────────── Admin transfer ───────────────────────

#[test]
fn admin_transfer_flow() {
    let (env, client, _registry, admin, _approver, _token, _ta) = setup();
    let new_admin = Address::generate(&env);

    client.propose_admin(&new_admin);
    client.accept_admin();

    assert_eq!(client.get_admin(), Ok(new_admin.clone()));

    let expected_event = AdminTransferred {
        previous_admin: admin,
        new_admin,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn accept_admin_with_no_pending_proposal_fails() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();

    let result = client.try_accept_admin();
    assert_eq!(result, Err(Ok(Error::NoPendingTransfer)));
}

#[test]
fn propose_admin_by_non_admin_fails() {
    let (env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (new_admin,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_propose_admin(&new_admin);
    assert!(result.is_err());
}

// ─────────────────────── Multi-claim flow ───────────────────────

#[test]
fn multiple_work_items_single_attester() {
    let (env, client, attester_registry, _admin, _approver, token, _token_admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let total = 5 * ONE_UNIT;
    client.fund(&total);

    for i in 0..5u8 {
        let id = BytesN::from_array(&env, &[i; 32]);
        client.approve_work_item(&id, &attester, &ONE_UNIT);
        client.claim(&id);
    }

    assert_eq!(client.get_total_paid(), Ok(total));
    assert_eq!(client.get_attester_total_claimed(attester.clone()), total);

    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&attester), total);
}

#[test]
fn multiple_attesters_independent_caps() {
    let (env, client, attester_registry, _admin, _approver, _token, _ta) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);

    client.fund(&(2 * POOL_CAP));

    let id_a = BytesN::from_array(&env, &[1u8; 32]);
    let id_b = BytesN::from_array(&env, &[2u8; 32]);

    client.approve_work_item(&id_a, &attester_a, &POOL_CAP);
    client.approve_work_item(&id_b, &attester_b, &POOL_CAP);

    client.claim(&id_a);
    client.claim(&id_b);

    assert_eq!(client.get_attester_total_claimed(attester_a), POOL_CAP);
    assert_eq!(client.get_attester_total_claimed(attester_b), POOL_CAP);
    assert_eq!(client.get_total_paid(), Ok(2 * POOL_CAP));
    let _ = env;
}

// ─────────────────────── Before initialization ───────────────────────

#[test]
fn getters_before_initialize_fail() {
    let env = Env::default();
    let contract_id = env.register(IncentivePool, ());
    let client = IncentivePoolClient::new(&env, &contract_id);

    assert_eq!(client.try_get_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_get_approver(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_get_token(), Err(Ok(Error::NotInitialized)));
    assert_eq!(
        client.try_get_attester_registry(),
        Err(Ok(Error::NotInitialized))
    );
}

// ─────────────────────── Error codes & events documentation ───────────────────────

fn parse_error_variants(content: &str) -> std::vec::Vec<std::string::String> {
    let mut variants = std::vec::Vec::new();
    if let Some(start_idx) = content.find("pub enum Error") {
        if let Some(block_start) = content[start_idx..].find('{') {
            let block = &content[start_idx + block_start + 1..];
            if let Some(block_end) = block.find('}') {
                let body = &block[..block_end];
                for line in body.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with("//") {
                        continue;
                    }
                    if let Some(first_char) = line.chars().next() {
                        if first_char.is_ascii_alphabetic() {
                            let name: std::string::String = line
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric())
                                .collect();
                            if !name.is_empty() {
                                variants.push(name);
                            }
                        }
                    }
                }
            }
        }
    }
    variants
}

fn parse_contract_events(content: &str) -> std::vec::Vec<std::string::String> {
    let mut events = std::vec::Vec::new();
    let mut event_attribute_seen = false;

    for line in content.lines() {
        let line = line.trim();
        if line == "#[contractevent]" {
            event_attribute_seen = true;
        } else if let (true, Some(declaration)) =
            (event_attribute_seen, line.strip_prefix("pub struct "))
        {
            let name = declaration
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            events.push(name);
            event_attribute_seen = false;
        }
    }

    events
}

fn markdown_section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let body = &content[start + heading.len()..];
    let end = body
        .find("\n## ")
        .map_or(content.len(), |offset| start + heading.len() + offset);
    Some(&content[start..end])
}

#[test]
fn test_error_codes_are_documented() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let doc_path = workspace_root.join("docs").join("error-codes.md");
    let doc_content = std::fs::read_to_string(&doc_path)
        .expect("Failed to read docs/error-codes.md. Make sure it exists.");

    let incentive_pool_src_path = workspace_root
        .join("contracts")
        .join("incentive-pool")
        .join("src")
        .join("lib.rs");
    let incentive_pool_src = std::fs::read_to_string(&incentive_pool_src_path)
        .expect("Failed to read incentive-pool lib.rs");

    let variants = parse_error_variants(&incentive_pool_src);
    assert!(
        !variants.is_empty(),
        "Could not find any Error variants in incentive-pool"
    );

    let heading = std::format!("## `incentive-pool`");
    let section = markdown_section(&doc_content, &heading)
        .unwrap_or_else(|| panic!("Missing '{heading}' section in docs/error-codes.md"));
    for variant in variants {
        assert!(
            section.contains(&std::format!("`{variant}`")),
            "Error variant '{variant}' is not documented under '{heading}' in docs/error-codes.md"
        );
    }
}

#[test]
fn test_contract_events_are_documented() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let doc_path = workspace_root
        .join("docs")
        .join("architecture")
        .join("event-indexing.md");
    let doc_content = std::fs::read_to_string(&doc_path)
        .expect("Failed to read docs/architecture/event-indexing.md. Make sure it exists.");

    let source_path = workspace_root
        .join("contracts")
        .join("incentive-pool")
        .join("src")
        .join("lib.rs");
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read incentive-pool lib.rs"));
    let events = parse_contract_events(&source);
    assert!(
        !events.is_empty(),
        "Could not find any contract events in incentive-pool"
    );

    for event in events {
        assert!(
            doc_content.contains(&std::format!("`{event}`")),
            "Event '{event}' from incentive-pool is not documented in docs/architecture/event-indexing.md"
        );
    }
}

// ─────────────────────── Work item info ───────────────────────

#[test]
fn get_work_item_returns_none_for_unknown() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let id = BytesN::from_array(&env, &[99u8; 32]);
    assert_eq!(client.get_work_item(&id), None);
    assert!(!client.is_work_item_approved(id));
    assert!(!client.is_work_item_claimed(id));
}

#[test]
fn attester_total_claimed_starts_at_zero() {
    let (_env, client, _registry, _admin, _approver, _token, _ta) = setup();
    let attester = Address::generate(&env);
    assert_eq!(client.get_attester_total_claimed(&attester), 0);
}
