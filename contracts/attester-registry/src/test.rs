#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Env, Event, IntoVal};

fn setup() -> (Env, AttesterRegistryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    (env, client, admin)
}

#[test]
fn initialize_sets_admin() {
    let (_, client, admin) = setup();
    client.initialize(&admin);
}

#[test]
fn initialize_twice_fails() {
    let (_, client, admin) = setup();
    client.initialize(&admin);

    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn is_attester_false_before_allowlisting() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let someone = Address::generate(&env);
    assert!(!client.is_attester(&someone));
}

#[test]
fn add_attester_allowlists_and_emits_event() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "add_attester"),
                    (attester.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    let expected_event = AttesterAdded {
        attester: attester.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    assert!(client.is_attester(&attester));
}

#[test]
fn remove_attester_revokes_allowlisting() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    assert!(client.is_attester(&attester));

    client.remove_attester(&attester);
    assert!(!client.is_attester(&attester));
}

#[test]
fn remove_attester_never_added_is_a_no_op() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.remove_attester(&attester);
    assert!(!client.is_attester(&attester));
}

#[test]
fn add_attester_before_initialize_fails() {
    let (env, client, _admin) = setup();
    let attester = Address::generate(&env);

    let result = client.try_add_attester(&attester);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn add_attester_without_admin_auth_fails() {
    // No mock_all_auths(): calls must present a real, matching auth entry.
    let env = Env::default();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let attester = Address::generate(&env);

    env.mock_all_auths();
    client.initialize(&admin);

    // Only mock an auth entry for `attester`, not `admin`, so the
    // contract's `admin.require_auth()` has nothing to satisfy it.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_attester",
            args: (attester.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_add_attester(&attester);
    assert!(result.is_err());
    assert!(!client.is_attester(&attester));
}

#[test]
fn batch_add_and_remove_attesters_success() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester1 = Address::generate(&env);
    let attester2 = Address::generate(&env);
    let attester3 = Address::generate(&env);

    let mut batch = Vec::new(&env);
    batch.push_back(attester1.clone());
    batch.push_back(attester2.clone());
    batch.push_back(attester3.clone());

    // Clear events from setup / initialize
    env.events().all();

    // Add batch
    client.add_attesters(&batch);

    // Verify events immediately (before other contract calls clear the event log)
    let events = env.events().all().events().to_vec();
    assert_eq!(events.len(), 3);
    assert_eq!(*events.get(0).unwrap(), AttesterAdded { attester: attester1.clone() }.to_xdr(&env, &client.address));
    assert_eq!(*events.get(1).unwrap(), AttesterAdded { attester: attester2.clone() }.to_xdr(&env, &client.address));
    assert_eq!(*events.get(2).unwrap(), AttesterAdded { attester: attester3.clone() }.to_xdr(&env, &client.address));

    assert!(client.is_attester(&attester1));
    assert!(client.is_attester(&attester2));
    assert!(client.is_attester(&attester3));

    // Remove batch
    let mut remove_batch = Vec::new(&env);
    remove_batch.push_back(attester1.clone());
    remove_batch.push_back(attester2.clone());

    client.remove_attesters(&remove_batch);

    // Verify remove events immediately
    let events = env.events().all().events().to_vec();
    assert_eq!(events.len(), 2);
    assert_eq!(*events.get(0).unwrap(), AttesterRemoved { attester: attester1.clone() }.to_xdr(&env, &client.address));
    assert_eq!(*events.get(1).unwrap(), AttesterRemoved { attester: attester2.clone() }.to_xdr(&env, &client.address));

    assert!(!client.is_attester(&attester1));
    assert!(!client.is_attester(&attester2));
    assert!(client.is_attester(&attester3));
}

#[test]
fn batch_add_and_remove_exceeding_limit_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let mut batch = Vec::new(&env);
    for _ in 0..=(BATCH_LIMIT) {
        batch.push_back(Address::generate(&env));
    }

    let result_add = client.try_add_attesters(&batch);
    assert_eq!(result_add, Err(Ok(Error::BatchTooLarge)));

    let result_remove = client.try_remove_attesters(&batch);
    assert_eq!(result_remove, Err(Ok(Error::BatchTooLarge)));
}

#[test]
fn batch_add_and_remove_partial_failure_idempotent() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester1 = Address::generate(&env);
    let attester2 = Address::generate(&env);

    // Pre-add attester1
    client.add_attester(&attester1);
    assert!(client.is_attester(&attester1));

    // Clear events
    env.events().all();

    // Batch add both. attester1 is already allowlisted, so it should be skipped.
    let mut batch = Vec::new(&env);
    batch.push_back(attester1.clone());
    batch.push_back(attester2.clone());

    client.add_attesters(&batch);

    // Verify only attester2 event was emitted immediately
    let events = env.events().all().events().to_vec();
    assert_eq!(events.len(), 1);
    assert_eq!(*events.get(0).unwrap(), AttesterAdded { attester: attester2.clone() }.to_xdr(&env, &client.address));

    // Both should be allowlisted
    assert!(client.is_attester(&attester1));
    assert!(client.is_attester(&attester2));

    // Clear events
    env.events().all();

    // Pre-remove attester1
    client.remove_attester(&attester1);
    assert!(!client.is_attester(&attester1));
    assert!(client.is_attester(&attester2));

    // Clear events
    env.events().all();

    // Batch remove both. attester1 is not allowlisted, so it should be skipped.
    let mut remove_batch = Vec::new(&env);
    remove_batch.push_back(attester1.clone());
    remove_batch.push_back(attester2.clone());

    client.remove_attesters(&remove_batch);

    // Verify only attester2 event was emitted immediately
    let events = env.events().all().events().to_vec();
    assert_eq!(events.len(), 1);
    assert_eq!(*events.get(0).unwrap(), AttesterRemoved { attester: attester2.clone() }.to_xdr(&env, &client.address));

    assert!(!client.is_attester(&attester1));
    assert!(!client.is_attester(&attester2));
}

#[test]
fn test_batch_budget() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    // Build a maximum sized batch (50 addresses)
    let mut batch = Vec::new(&env);
    for _ in 0..BATCH_LIMIT {
        batch.push_back(Address::generate(&env));
    }

    // Reset CPU budget tracking to get clean metrics
    env.cost_estimate().budget().reset_default();

    // Run maximum batch addition
    client.add_attesters(&batch);

    // Retrieve CPU instruction count
    let cpu_insns = env.cost_estimate().budget().cpu_instruction_cost();
    std::println!("CPU instructions consumed for batch of {}: {}", BATCH_LIMIT, cpu_insns);

    // Assert that the resource consumption is well under Soroban's per-transaction limit (100 million)
    assert!(cpu_insns < 100_000_000, "CPU instructions exceed 100M limit");
}
