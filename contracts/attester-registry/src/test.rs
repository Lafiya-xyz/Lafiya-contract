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

fn ensure_wasm_built() {
    let wasm_path =
        std::path::Path::new("../../target/wasm32v1-none/release/attester_registry.wasm");
    if !wasm_path.exists() {
        let output = std::process::Command::new("cargo")
            .args([
                "build",
                "--workspace",
                "--release",
                "--target",
                "wasm32v1-none",
            ])
            .current_dir("../..")
            .output()
            .expect("Failed to execute cargo build");
        if !output.status.success() {
            panic!(
                "Cargo build failed: {}",
                std::string::String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn test_contract_upgrade_flow() {
    ensure_wasm_built();

    let env = Env::default();
    env.mock_all_auths();

    let old_wasm = std::fs::read("../../target/wasm32v1-none/release/attester_registry.wasm")
        .expect("Failed to read old WASM");
    let new_wasm = std::fs::read("../../target/wasm32v1-none/release/attestation_registry.wasm")
        .expect("Failed to read new WASM");

    let old_wasm_bytes = soroban_sdk::Bytes::from_slice(&env, &old_wasm);
    let new_wasm_bytes = soroban_sdk::Bytes::from_slice(&env, &new_wasm);

    let new_wasm_hash = env.deployer().upload_contract_wasm(new_wasm_bytes);
    let contract_id = env.register_contract_wasm(None, old_wasm_bytes);
    let client = AttesterRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    assert!(client.is_attester(&attester));

    // Upgrade the contract Wasm
    client.upgrade(&new_wasm_hash);

    // Verify event
    let expected_event = Upgraded {
        new_wasm_hash: new_wasm_hash.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)]
    );

    // Confirm that the address behaves like AttestationRegistry now (e.g. initialize has 2 args)
    let dummy_registry = Address::generate(&env);
    let result = env.try_invoke_contract::<(), soroban_sdk::Error>(
        &contract_id,
        &soroban_sdk::Symbol::new(&env, "initialize"),
        (admin, dummy_registry).into_val(&env),
    );
    // Should fail with AlreadyInitialized (contract error 2)
    assert!(result.is_err());
}

#[test]
fn test_upgrade_without_admin_fails() {
    ensure_wasm_built();

    let env = Env::default();
    let old_wasm = std::fs::read("../../target/wasm32v1-none/release/attester_registry.wasm")
        .expect("Failed to read old WASM");
    let new_wasm = std::fs::read("../../target/wasm32v1-none/release/attestation_registry.wasm")
        .expect("Failed to read new WASM");

    let old_wasm_bytes = soroban_sdk::Bytes::from_slice(&env, &old_wasm);
    let new_wasm_bytes = soroban_sdk::Bytes::from_slice(&env, &new_wasm);

    let new_wasm_hash = env.deployer().upload_contract_wasm(new_wasm_bytes);
    let contract_id = env.register_contract_wasm(None, old_wasm_bytes);
    let client = AttesterRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    // Disable mock auth to test rejection
    env.mock_auths(&[]);
    let result = client.try_upgrade(&new_wasm_hash);
    assert!(result.is_err());
}
