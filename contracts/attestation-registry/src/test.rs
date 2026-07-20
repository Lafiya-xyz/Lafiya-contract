extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{BytesN, Env, Event, IntoVal};

fn setup() -> (
    Env,
    AttestationRegistryClient<'static>,
    attester_registry::AttesterRegistryClient<'static>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    attester_registry_client.initialize(&admin);
    client.initialize(&admin, &attester_registry_id);

    (env, client, attester_registry_client, admin)
}

#[test]
fn attest_by_allowlisted_attester_succeeds() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[7u8; 32]);
    let attestation = client.attest(&attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(client.get_attestation(&record_hash), Some(attestation));
}

#[test]
fn attest_by_non_allowlisted_attester_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[1u8; 32]);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn attest_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[2u8; 32]);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn get_attestation_returns_none_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[9u8; 32]);
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn re_attest_overwrites_previous_attestation() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);

    let record_hash = BytesN::from_array(&env, &[3u8; 32]);
    client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);

    assert_eq!(client.get_attestation(&record_hash), Some(second));
}

#[test]
fn attest_emits_event() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[4u8; 32]);

    let attestation = client.attest(&attester, &record_hash);

    let expected_event = AttestationRecorded {
        record_hash: record_hash.clone(),
        attester: attestation.attester.clone(),
        timestamp: attestation.timestamp,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );
}

#[test]
fn attest_without_attester_auth_fails() {
    let (env, client, attester_registry, admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);
    let record_hash = BytesN::from_array(&env, &[5u8; 32]);
    let _ = &admin;

    env.mock_auths(&[]);
    let result = client.try_attest(&attester, &record_hash);
    assert!(result.is_err());
    assert_eq!(client.get_attestation(&record_hash), None);
}

#[test]
fn propose_admin_by_non_admin_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    client.initialize(&admin, &attester_registry_id);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (new_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_propose_admin(&new_admin);
    assert!(result.is_err());
}

#[test]
fn accept_admin_by_wrong_address_fails() {
    let env = Env::default();
    let contract_id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    client.initialize(&admin, &attester_registry_id);
    client.propose_admin(&new_admin);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_accept_admin();
    assert!(result.is_err());
}

#[test]
fn accept_admin_with_no_pending_proposal_fails() {
    let (_env, client, _, _admin) = setup();

    let result = client.try_accept_admin();
    assert_eq!(result, Err(Ok(Error::NoPendingTransfer)));
}

#[test]
fn successful_admin_transfer_flow() {
    let (env, client, _, admin) = setup();

    let new_admin = Address::generate(&env);

    client.propose_admin(&new_admin);

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "propose_admin"),
                    (new_admin.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    client.accept_admin();

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "accept_admin"),
                    ().into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    let expected_event = AdminTransferred {
        previous_admin: admin.clone(),
        new_admin: new_admin.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    let newer_admin = Address::generate(&env);
    client.propose_admin(&newer_admin);

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "propose_admin"),
                    (newer_admin.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (newer_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_propose_admin(&newer_admin);
    assert!(result.is_err());
}

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

    let attester_src_path = workspace_root
        .join("contracts")
        .join("attester-registry")
        .join("src")
        .join("lib.rs");
    let attester_src = std::fs::read_to_string(&attester_src_path)
        .expect("Failed to read attester-registry lib.rs");

    let attestation_src_path = workspace_root
        .join("contracts")
        .join("attestation-registry")
        .join("src")
        .join("lib.rs");
    let attestation_src = std::fs::read_to_string(&attestation_src_path)
        .expect("Failed to read attestation-registry lib.rs");

    let attester_variants = parse_error_variants(&attester_src);
    let attestation_variants = parse_error_variants(&attestation_src);

    assert!(
        !attester_variants.is_empty(),
        "Could not find any Error variants in attester-registry"
    );
    assert!(
        !attestation_variants.is_empty(),
        "Could not find any Error variants in attestation-registry"
    );

    let attester_section_idx = doc_content
        .find("## `attester-registry`")
        .expect("Missing '## `attester-registry`' section in docs/error-codes.md");
    let attestation_section_idx = doc_content
        .find("## `attestation-registry`")
        .expect("Missing '## `attestation-registry`' section in docs/error-codes.md");

    let (attester_doc, attestation_doc) = if attester_section_idx < attestation_section_idx {
        (
            &doc_content[attester_section_idx..attestation_section_idx],
            &doc_content[attestation_section_idx..],
        )
    } else {
        (
            &doc_content[attester_section_idx..],
            &doc_content[attestation_section_idx..attester_section_idx],
        )
    };

    for variant in &attester_variants {
        assert!(
            attester_doc.contains(variant),
            "Error variant '{}' is not documented under '## `attester-registry`' in docs/error-codes.md",
            variant
        );
    }

    for variant in &attestation_variants {
        assert!(
            attestation_doc.contains(variant),
            "Error variant '{}' is not documented under '## `attestation-registry`' in docs/error-codes.md",
            variant
        );
    }
}
