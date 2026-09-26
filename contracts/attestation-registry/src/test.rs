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
    let admin = Address::generate(&env);

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let attester_registry_client =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let contract_id = env.register(
        AttestationRegistry,
        (admin.clone(), attester_registry_id.clone()),
    );
    let client = AttestationRegistryClient::new(&env, &contract_id);

    (env, client, attester_registry_client, admin)
}

#[test]
fn configuration_getters_return_initialized_addresses() {
    let (_env, client, attester_registry, admin) = setup();

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_attester_registry(), attester_registry.address);
}

#[test]
fn attest_by_allowlisted_attester_succeeds() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[7u8; 32]);
    let attestation = client.attest(&attester, &record_hash);

    assert_eq!(attestation.attester, attester);
    assert_eq!(
        client.get_attestation(&record_hash),
        soroban_sdk::Vec::from_array(&env, [attestation])
    );
}

#[test]
fn attest_by_non_allowlisted_attester_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[1u8; 32]);

    let result = client.try_attest(&attester, &record_hash);
    assert_eq!(result, Err(Ok(Error::AttesterNotAllowlisted)));
    assert!(client.get_attestation(&record_hash).is_empty());
}

#[test]
fn get_attestation_returns_empty_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[9u8; 32]);
    assert!(client.get_attestation(&record_hash).is_empty());
}

#[test]
fn distinct_attesters_are_kept_in_lookup() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);

    let record_hash = BytesN::from_array(&env, &[3u8; 32]);
    let first = client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);

    assert_eq!(
        client.get_attestation(&record_hash),
        soroban_sdk::Vec::from_array(&env, [first.clone(), second.clone()])
    );

    // Both attesters remain visible in the primary lookup and history.
    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0), Some(first));
    assert_eq!(history.get(1), Some(second));
}

#[test]
fn repeated_attestations_do_not_evict_other_attesters() {
    let (env, client, attester_registry, _admin) = setup();
    let clinician = Address::generate(&env);
    let repeated_attester = Address::generate(&env);
    attester_registry.add_attester(&clinician);
    attester_registry.add_attester(&repeated_attester);

    let record_hash = BytesN::from_array(&env, &[12u8; 32]);
    let clinician_attestation = client.attest(&clinician, &record_hash);
    for _ in 0..10 {
        client.attest(&repeated_attester, &record_hash);
    }

    let attestations = client.get_attestation(&record_hash);
    assert_eq!(attestations.len(), 2);
    assert_eq!(attestations.get(0), Some(clinician_attestation));
    assert_eq!(
        attestations.get(1).map(|attestation| attestation.attester),
        Some(repeated_attester)
    );
}

#[test]
fn get_attestation_history_returns_all_attestations() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    let attester_c = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);
    attester_registry.add_attester(&attester_c);

    let record_hash = BytesN::from_array(&env, &[6u8; 32]);
    let first = client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);
    let third = client.attest(&attester_c, &record_hash);

    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 3);
    assert_eq!(history.get(0), Some(first));
    assert_eq!(history.get(1), Some(second));
    assert_eq!(history.get(2), Some(third));
}

#[test]
fn get_attestation_history_returns_empty_for_unknown_hash() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[8u8; 32]);
    let history = client.get_attestation_history(&record_hash);
    assert_eq!(history.len(), 0);
}

#[test]
fn attestation_history_is_bounded() {
    let (env, client, attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[10u8; 32]);

    // Create more distinct attestations than MAX_HISTORY (10).
    for _ in 0..15 {
        let attester = Address::generate(&env);
        attester_registry.add_attester(&attester);
        client.attest(&attester, &record_hash);
    }

    let history = client.get_attestation_history(&record_hash);
    // Should only keep the last 10 attestations
    assert_eq!(history.len(), 10);
}

#[test]
fn get_attestation_history_boundary_at_max_history() {
    let (env, client, attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[11u8; 32]);

    // Attest exactly MAX_HISTORY times (10), then once more (11 total).
    // This tests the boundary where the oldest entry should be evicted.
    let mut attestations = std::vec![];
    for _ in 0..11 {
        let attester = Address::generate(&env);
        attester_registry.add_attester(&attester);
        attestations.push(client.attest(&attester, &record_hash));
    }

    let history = client.get_attestation_history(&record_hash);

    // History should contain exactly MAX_HISTORY entries (10).
    // The first attestation (oldest) should have been evicted.
    assert_eq!(
        history.len(),
        10,
        "Expected exactly MAX_HISTORY entries in history"
    );

    // The oldest retained slot should be the second attestation.
    assert_eq!(
        history.get(0).unwrap().timestamp,
        attestations[1].timestamp,
        "Oldest entry in history should be the 2nd attestation (1st was evicted)"
    );

    // The newest entry in history should be the last (11th) attestation.
    assert_eq!(
        history.get(9).unwrap().timestamp,
        attestations[10].timestamp,
        "Newest entry in history should be the 11th attestation"
    );

    // Verify all 10 entries are from attestations 2-11 (in order).
    for i in 0..10 {
        assert_eq!(
            history.get(i).unwrap().timestamp,
            attestations.get(i as usize + 1).unwrap().timestamp,
            "History entry {} should match attestation {}",
            i,
            i + 1
        );
    }
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
    assert!(client.get_attestation(&record_hash).is_empty());
}

#[test]
fn propose_admin_by_non_admin_fails() {
    let (env, client, _attester_registry, _admin) = setup();
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

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
    let (env, client, _attester_registry, _admin) = setup();
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

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

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_non_contract_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let non_contract = Address::generate(&env);
    env.register(AttestationRegistry, (admin, non_contract));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_contract_without_attester_interface() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let attester_registry = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let unrelated_contract = env.register(AttestationRegistry, (admin.clone(), attester_registry));
    env.register(AttestationRegistry, (admin, unrelated_contract));
}

#[test]
fn constructor_accepts_real_attester_registry() {
    let (_env, client, attester_registry, admin) = setup();
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_attester_registry(), attester_registry.address);
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
    let multisig_src_path = workspace_root
        .join("contracts")
        .join("multisig-account")
        .join("src")
        .join("lib.rs");
    let multisig_src = std::fs::read_to_string(&multisig_src_path)
        .expect("Failed to read multisig-account lib.rs");

    for (contract, source) in [
        ("attester-registry", attester_src.as_str()),
        ("attestation-registry", attestation_src.as_str()),
        ("multisig-account", multisig_src.as_str()),
    ] {
        let variants = parse_error_variants(source);
        assert!(
            !variants.is_empty(),
            "Could not find any Error variants in {contract}"
        );

        let heading = std::format!("## `{contract}`");
        let section = markdown_section(&doc_content, &heading)
            .unwrap_or_else(|| panic!("Missing '{heading}' section in docs/error-codes.md"));
        for variant in variants {
            assert!(
                section.contains(&std::format!("`{variant}`")),
                "Error variant '{variant}' is not documented under '{heading}' in docs/error-codes.md"
            );
        }
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

    for contract in ["attester-registry", "attestation-registry"] {
        let source_path = workspace_root
            .join("contracts")
            .join(contract)
            .join("src")
            .join("lib.rs");
        let source = std::fs::read_to_string(&source_path)
            .unwrap_or_else(|_| panic!("Failed to read {contract} lib.rs"));
        let events = parse_contract_events(&source);
        assert!(
            !events.is_empty(),
            "Could not find any contract events in {contract}"
        );

        for event in events {
            assert!(
                doc_content.contains(&std::format!("`{event}`")),
                "Event '{event}' from {contract} is not documented in docs/architecture/event-indexing.md"
            );
        }
    }
}

#[test]
fn test_attest_auth_matrix() {
    struct TestCase {
        name: &'static str,
        call_role: &'static str, // "attester", "wrong_user", "admin"
        auth_role: &'static str, // "attester", "wrong_user", "admin", "none"
        allowlisted: bool,
        expected_result: Result<
            Result<Attestation, soroban_sdk::ConversionError>,
            Result<Error, soroban_sdk::InvokeError>,
        >,
    }

    let cases = std::vec![
        TestCase {
            name: "Right Caller (Attester)",
            call_role: "attester",
            auth_role: "attester",
            allowlisted: true,
            expected_result: Ok(Ok(Attestation {
                attester: Address::generate(&Env::default()), // will be overwritten in comparison/check
                timestamp: 0,
            })),
        },
        TestCase {
            name: "Wrong Caller (not allowlisted)",
            call_role: "wrong_user",
            auth_role: "wrong_user",
            allowlisted: false,
            expected_result: Err(Ok(Error::AttesterNotAllowlisted)),
        },
        TestCase {
            name: "Wrong Caller (wrong auth)",
            call_role: "attester",
            auth_role: "wrong_user",
            allowlisted: true,
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "No Auth Provided",
            call_role: "attester",
            auth_role: "none",
            allowlisted: true,
            expected_result: Err(Err(soroban_sdk::InvokeError::Abort)),
        },
        TestCase {
            name: "Role Confusion (Admin)",
            call_role: "admin",
            auth_role: "admin",
            allowlisted: false,
            expected_result: Err(Ok(Error::AttesterNotAllowlisted)),
        },
    ];

    for case in cases {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let attester_registry_id =
            env.register(attester_registry::AttesterRegistry, (admin.clone(),));
        let attester_registry_client =
            attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

        let contract_id = env.register(
            AttestationRegistry,
            (admin.clone(), attester_registry_id.clone()),
        );
        let client = AttestationRegistryClient::new(&env, &contract_id);

        let wrong_user = Address::generate(&env);
        let attester = Address::generate(&env);
        let record_hash = BytesN::from_array(&env, &[99u8; 32]);

        if case.allowlisted {
            attester_registry_client.add_attester(&attester);
        }

        // Determine which addresses are used for call vs auth
        let call_address = match case.call_role {
            "attester" => attester.clone(),
            "wrong_user" => wrong_user.clone(),
            "admin" => admin.clone(),
            _ => panic!("Unknown call role"),
        };

        let auth_address = match case.auth_role {
            "attester" => Some(attester.clone()),
            "wrong_user" => Some(wrong_user.clone()),
            "admin" => Some(admin.clone()),
            _ => None,
        };

        if let Some(addr) = auth_address {
            env.mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &addr,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &client.address,
                    fn_name: "attest",
                    args: (call_address.clone(), record_hash.clone()).into_val(&env),
                    sub_invokes: &[],
                },
            }]);
        } else {
            env.mock_auths(&[]);
        }

        let result = client.try_attest(&call_address, &record_hash);

        // Check matching expected results
        match (&result, &case.expected_result) {
            (Ok(Ok(attestation)), Ok(Ok(_))) => {
                assert_eq!(attestation.attester, call_address);
                assert!(client
                    .get_attestation(&record_hash)
                    .iter()
                    .any(|stored| stored == *attestation));
            }
            (Err(Ok(err)), Err(Ok(expected_err))) => {
                assert_eq!(err, expected_err);
                assert!(client.get_attestation(&record_hash).is_empty());
            }
            (
                Err(Err(soroban_sdk::InvokeError::Abort)),
                Err(Err(soroban_sdk::InvokeError::Abort)),
            ) => {
                assert!(client.get_attestation(&record_hash).is_empty());
            }
            _ => panic!(
                "Failed case '{}': expected {:?}, got {:?}",
                case.name, case.expected_result, result
            ),
        }
    }
}

#[test]
fn set_attester_registry_by_admin_succeeds() {
    let (env, client, attester_registry, admin) = setup();

    let new_registry = Address::generate(&env);
    assert_eq!(client.get_attester_registry(), attester_registry.address);

    client.set_attester_registry(&new_registry);

    // Check event was emitted before any other call clears it
    let expected_event = AttesterRegistryRepointed {
        previous: attester_registry.address.clone(),
        new: new_registry.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "set_attester_registry"),
                    (new_registry.clone(),).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    assert_eq!(client.get_attester_registry(), new_registry);
}

#[test]
fn set_attester_registry_by_non_admin_fails() {
    let (env, client, attester_registry, _admin) = setup();
    let malicious = Address::generate(&env);
    let new_registry = Address::generate(&env);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_attester_registry",
            args: (new_registry.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_set_attester_registry(&new_registry);
    assert!(result.is_err());
    // Registry should be unchanged
    assert_eq!(client.get_attester_registry(), attester_registry.address);
}

#[test]
fn revoke_attestation_happy_path() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[11u8; 32]);
    client.attest(&attester, &record_hash);
    assert!(!client.get_attestation(&record_hash).is_empty());

    client.revoke_attestation(&record_hash);

    assert!(client.get_attestation(&record_hash).is_empty());
}

#[test]
fn revoke_attestation_without_admin_auth_fails() {
    let (env, client, attester_registry, _admin) = setup();
    let attester = Address::generate(&env);
    let malicious = Address::generate(&env);
    attester_registry.add_attester(&attester);

    let record_hash = BytesN::from_array(&env, &[12u8; 32]);
    let attestation = client.attest(&attester, &record_hash);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &malicious,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "revoke_attestation",
            args: (record_hash.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_revoke_attestation(&record_hash);
    assert!(result.is_err());
    assert!(client
        .get_attestation(&record_hash)
        .iter()
        .any(|stored| stored == attestation));
}

#[test]
fn revoke_attestation_for_unknown_hash_returns_not_initialized() {
    let (env, client, _attester_registry, _admin) = setup();
    let record_hash = BytesN::from_array(&env, &[13u8; 32]);

    let result = client.try_revoke_attestation(&record_hash);
    // NOTE: This is a bug in the contract. revoke_attestation returns
    // Error::NotInitialized when called with an unknown hash (line 371 in lib.rs),
    // but it should return Error::AttestationNotFound. This test documents
    // the actual current behavior, not the intended behavior.
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn revoke_attestation_clears_get_attestation_history() {
    let (env, client, attester_registry, _admin) = setup();
    let attester_a = Address::generate(&env);
    let attester_b = Address::generate(&env);
    let attester_c = Address::generate(&env);
    attester_registry.add_attester(&attester_a);
    attester_registry.add_attester(&attester_b);
    attester_registry.add_attester(&attester_c);

    let record_hash = BytesN::from_array(&env, &[14u8; 32]);
    let first = client.attest(&attester_a, &record_hash);
    let second = client.attest(&attester_b, &record_hash);
    let third = client.attest(&attester_c, &record_hash);

    let history_before = client.get_attestation_history(&record_hash);
    assert_eq!(history_before.len(), 3);
    assert_eq!(history_before.get(0), Some(first));
    assert_eq!(history_before.get(1), Some(second));
    assert_eq!(history_before.get(2), Some(third));

    assert_eq!(client.get_attestation(&record_hash), history_before);

    client.revoke_attestation(&record_hash);

    assert!(client.get_attestation(&record_hash).is_empty());
    let history_after = client.get_attestation_history(&record_hash);
    assert_eq!(history_after.len(), 0);
}
