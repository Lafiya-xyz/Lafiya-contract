#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{BytesN, Env, Event, IntoVal};

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
fn get_schema_version_is_zero_before_initialize() {
    let (_, client, _admin) = setup();
    assert_eq!(client.get_schema_version(), 0);
}

#[test]
fn initialize_records_schema_version_one() {
    let (_, client, admin) = setup();
    client.initialize(&admin);
    assert_eq!(client.get_schema_version(), 1);
}

#[test]
fn upgrade_before_initialize_fails() {
    let (env, client, _admin) = setup();
    let wasm_hash = BytesN::from_array(&env, &[8u8; 32]);

    let result = client.try_upgrade(&wasm_hash);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn upgrade_without_admin_auth_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    // Replace the blanket auth mock with an empty set: the upgrade call's
    // `admin.require_auth()` now has no matching auth entry to satisfy it.
    env.mock_auths(&[]);
    let wasm_hash = BytesN::from_array(&env, &[9u8; 32]);
    let result = client.try_upgrade(&wasm_hash);
    assert!(result.is_err());
}

#[test]
fn migrate_before_initialize_fails() {
    let (_, client, _admin) = setup();

    let result = client.try_migrate();
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn migrate_fails_when_no_migration_pending() {
    let (_, client, admin) = setup();
    client.initialize(&admin);

    // initialize already recorded SCHEMA_VERSION, so there is nothing to do.
    let result = client.try_migrate();
    assert_eq!(result, Err(Ok(Error::MigrationNotRequired)));
}

#[test]
fn migrate_without_admin_auth_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);
    // Simulate a legacy (pre-versioning) instance so a migration IS pending.
    env.as_contract(&client.address, || {
        env.storage().instance().remove(&DataKey::SchemaVersion);
    });

    env.mock_auths(&[]);
    let result = client.try_migrate();
    assert!(result.is_err());

    env.mock_all_auths();
    assert_eq!(client.get_schema_version(), 0);
}

#[test]
fn migrate_records_version_and_preserves_data() {
    let (env, client, admin) = setup();
    client.initialize(&admin);
    let attester = Address::generate(&env);
    client.add_attester(&attester);

    // Simulate a legacy (pre-versioning) instance: instance storage was
    // written before the SchemaVersion key existed.
    env.as_contract(&client.address, || {
        env.storage().instance().remove(&DataKey::SchemaVersion);
    });
    assert_eq!(client.get_schema_version(), 0);
    assert!(client.is_attester(&attester));

    client.migrate();

    assert_eq!(client.get_schema_version(), 1);
    assert!(client.is_attester(&attester));
}
