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

fn initialize_for_tests(client: &AttesterRegistryClient, admin: &Address) {
    client.initialize(admin);
    client.grant_role(&Role::Registrar, admin);
    client.grant_role(&Role::Guardian, admin);
}

#[test]
fn get_schema_version_succeeds() {
    // Asserts the literal current schema version, not just that the call
    // succeeds. Any change to this expected value is a schema version bump
    // and must be deliberate, paired with a migration plan (see
    // `needs_migration`/`migrate` in lib.rs), and not an accidental side
    // effect of an unrelated change.
    let (_, client, admin) = setup();
    assert_eq!(client.get_schema_version(), 1);
    initialize_for_tests(&client, &admin);
    assert_eq!(client.get_schema_version(), 4);
}

#[test]
fn initialize_sets_admin() {
    let (_, client, admin) = setup();
    initialize_for_tests(&client, &admin);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn get_admin_before_initialize_fails() {
    let (_, client, _admin) = setup();

    let result = client.try_get_admin();
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn initialize_twice_fails() {
    let (_, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn is_attester_false_before_allowlisting() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let someone = Address::generate(&env);
    assert!(!client.is_attester(&someone));
}

#[test]
fn add_attester_allowlists_and_emits_event() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);

    assert_eq!(
        env.auths(),
        std::vec![(
            admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "add_attester"),
                    (admin.clone(), attester.clone()).into_val(&env),
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
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    assert!(client.is_attester(&attester));

    client.remove_attester(&admin, &attester);
    assert!(!client.is_attester(&attester));
}

#[test]
fn remove_attester_never_added_is_a_no_op() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.remove_attester(&admin, &attester);
    assert!(!client.is_attester(&attester));
}

#[test]
fn add_attester_before_initialize_fails() {
    let (env, client, _admin) = setup();
    let attester = Address::generate(&env);

    let result = client.try_add_attester(&_admin, &attester);
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
    initialize_for_tests(&client, &admin);

    // Only mock an auth entry for `attester`, not `admin`, so the registrar's
    // require_auth() has nothing to satisfy it.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_attester",
            args: (admin.clone(), attester.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_add_attester(&admin, &attester);
    assert_eq!(result, Err(Err(soroban_sdk::InvokeError::Abort)));
    assert!(!client.is_attester(&attester));
}

#[test]
fn propose_admin_by_non_admin_fails() {
    let env = Env::default();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    initialize_for_tests(&client, &admin);

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
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let malicious = Address::generate(&env);

    env.mock_all_auths();
    initialize_for_tests(&client, &admin);
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
    let (_env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let result = client.try_accept_admin();
    assert_eq!(result, Err(Ok(Error::NoPendingTransfer)));
}

#[test]
fn successful_admin_transfer_flow() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

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

    client.revoke_role(&Role::Registrar, &admin);
    client.grant_role(&Role::Registrar, &new_admin);
    let attester = Address::generate(&env);
    client.add_attester(&new_admin, &attester);

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
            soroban_sdk::testutils::AuthorizedInvocation {
                function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                    client.address.clone(),
                    soroban_sdk::Symbol::new(&env, "add_attester"),
                    (new_admin.clone(), attester.clone()).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )]
    );

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_attester",
            args: (admin.clone(), attester.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_add_attester(&admin, &attester);
    assert!(result.is_err());
}

#[test]
fn add_attester_beyond_cap_fails() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);
    client.set_max_attesters(&2);

    client.add_attester(&admin, &Address::generate(&env));
    client.add_attester(&admin, &Address::generate(&env));
    assert_eq!(client.get_attester_count(), 2);

    let result = client.try_add_attester(&admin, &Address::generate(&env));
    assert_eq!(result, Err(Ok(Error::AllowlistFull)));
    assert_eq!(client.get_attester_count(), 2);
}

#[test]
fn removing_an_attester_frees_cap_slot() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);
    client.set_max_attesters(&1);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    assert_eq!(
        client.try_add_attester(&admin, &Address::generate(&env)),
        Err(Ok(Error::AllowlistFull))
    );

    client.remove_attester(&admin, &attester);
    assert_eq!(client.get_attester_count(), 0);
    client.add_attester(&admin, &Address::generate(&env));
    assert_eq!(client.get_attester_count(), 1);
}

#[test]
fn re_adding_an_existing_attester_does_not_consume_cap() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);
    client.set_max_attesters(&1);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.add_attester(&admin, &attester);
    assert_eq!(client.get_attester_count(), 1);
}

#[test]
fn update_attester_info_on_unknown_attester_fails() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    let license_hash = BytesN::from_array(&env, &[1u8; 32]);
    let region = Symbol::new(&env, "west");
    let result = client.try_update_attester_info(
        &admin,
        &attester,
        &Some(license_hash),
        &Some(region),
        &None,
        &None,
    );
    assert_eq!(result, Err(Ok(Error::AttesterNotFound)));
}

#[test]
fn update_attester_info_on_removed_attester_fails() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.remove_attester(&admin, &attester);

    let result = client.try_update_attester_info(&admin, &attester, &None, &None, &None, &None);
    assert_eq!(result, Err(Ok(Error::AttesterNotFound)));
}

#[test]
fn update_attester_info_updates_metadata_and_emits_distinct_event() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    let initial_hash = BytesN::from_array(&env, &[1u8; 32]);
    let initial_region = Symbol::new(&env, "west");
    client.add_attester_with_info(
        &admin,
        &attester,
        &Some(initial_hash),
        &Some(initial_region),
        &None,
        &None,
    );

    // Check event was emitted before any other call clears it.
    let expected_added_event = AttesterAdded {
        attester: attester.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_added_event.to_xdr(&env, &client.address)],
    );

    let updated_hash = BytesN::from_array(&env, &[2u8; 32]);
    let updated_region = Symbol::new(&env, "east");
    client.update_attester_info(
        &admin,
        &attester,
        &Some(updated_hash.clone()),
        &Some(updated_region.clone()),
        &Some(100),
        &Some(200),
    );

    let expected_updated_event = AttesterInfoUpdated {
        attester: attester.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_updated_event.to_xdr(&env, &client.address)],
    );

    assert_eq!(
        client.get_attester_info(&attester),
        Some(AttesterInfo {
            license_hash: Some(updated_hash),
            region: Some(updated_region),
            valid_from: Some(100),
            valid_until: Some(200),
        }),
    );
}

#[test]
fn update_attester_info_without_admin_auth_fails() {
    let env = Env::default();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let attester = Address::generate(&env);

    env.mock_all_auths();
    initialize_for_tests(&client, &admin);
    client.add_attester(&admin, &attester);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "update_attester_info",
            args: (
                admin.clone(),
                attester.clone(),
                None::<BytesN<32>>,
                None::<Symbol>,
                None::<u64>,
                None::<u64>,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_update_attester_info(&admin, &attester, &None, &None, &None, &None);
    assert_eq!(result, Err(Err(soroban_sdk::InvokeError::Abort)));
}

#[test]
fn update_attester_info_while_paused_fails() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.pause(&admin);

    let result = client.try_update_attester_info(&admin, &attester, &None, &None, &None, &None);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn get_attester_status_for_unknown_attester_is_none() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    assert_eq!(client.get_attester_status(&attester), None);
}

#[test]
fn get_attester_status_for_removed_attester_is_none() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    client.add_attester(&admin, &attester);
    client.remove_attester(&admin, &attester);

    assert_eq!(client.get_attester_status(&attester), None);
}

#[test]
fn lowering_max_attesters_below_current_count_does_not_evict() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    // Add 3 attesters with no cap restriction.
    let attester1 = Address::generate(&env);
    let attester2 = Address::generate(&env);
    let attester3 = Address::generate(&env);
    client.add_attester(&admin, &attester1);
    client.add_attester(&admin, &attester2);
    client.add_attester(&admin, &attester3);
    assert_eq!(client.get_attester_count(), 3);

    // Lower the cap to 1 — well below the current count of 3.
    client.set_max_attesters(&1);
    assert_eq!(client.get_max_attesters(), 1);

    // All previously-added attesters must still be active — no eviction.
    assert!(client.is_attester(&attester1));
    assert!(client.is_attester(&attester2));
    assert!(client.is_attester(&attester3));
    assert_eq!(client.get_attester_count(), 3);

    // Adding a new attester must fail with AllowlistFull because count >= cap.
    let new_attester = Address::generate(&env);
    let result = client.try_add_attester(&admin, &new_attester);
    assert_eq!(result, Err(Ok(Error::AllowlistFull)));
    assert!(!client.is_attester(&new_attester));
}

/// Calling `suspend_attester` on an address that was never allowlisted is a
/// no-op from an access-control perspective: the `Suspended` key is written
/// for that address and `AttesterSuspended` is emitted, but `is_attester`
/// still returns `false` because there is no matching `Attester` storage
/// entry. This inconsistency with `update_attester_info` (which returns
/// `Error::AttesterNotFound`) is documented on the function and tracked as a
/// known issue.
#[test]
fn suspend_unknown_attester_behavior() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let never_added = Address::generate(&env);

    // Precondition: the address has never been allowlisted.
    assert!(!client.is_attester(&never_added));

    // suspend_attester succeeds (no error) even though the address was never added.
    client.suspend_attester(&admin, &never_added);

    // The AttesterSuspended event was still emitted, confirming the call succeeded.
    let expected_event = AttesterSuspended {
        attester: never_added.clone(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected_event.to_xdr(&env, &client.address)],
    );

    // The phantom suspension has no effect on allowlist queries because
    // is_attester also checks for the Attester storage entry.
    assert!(!client.is_attester(&never_added));

    // get_attester_status returns None because there is no Attester entry.
    assert_eq!(client.get_attester_status(&never_added), None);
}

#[test]
fn get_attester_status_reports_metadata_and_suspension_consistently() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    let license_hash = BytesN::from_array(&env, &[3u8; 32]);
    let region = Symbol::new(&env, "north");
    client.add_attester_with_info(
        &admin,
        &attester,
        &Some(license_hash.clone()),
        &Some(region.clone()),
        &Some(0),
        &Some(1),
    );

    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash.clone()),
                region: Some(region.clone()),
                valid_from: Some(0),
                valid_until: Some(1),
            },
            suspended: false,
            status: AttesterStatusKind::Active,
        }),
    );
    assert!(client.is_attester(&attester));

    client.suspend_attester(&admin, &attester);
    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash.clone()),
                region: Some(region.clone()),
                valid_from: Some(0),
                valid_until: Some(1),
            },
            suspended: true,
            status: AttesterStatusKind::Suspended,
        }),
    );
    assert!(!client.is_attester(&attester));

    client.reinstate_attester(&admin, &attester);
    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash),
                region: Some(region),
                valid_from: Some(0),
                valid_until: Some(1),
            },
            suspended: false,
            status: AttesterStatusKind::Active,
        }),
    );
    assert!(client.is_attester(&attester));
}

#[test]
fn validity_window_controls_authorization_and_status() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let future_attester = Address::generate(&env);
    client.add_attester_with_info(
        &admin,
        &future_attester,
        &None,
        &Some(Symbol::new(&env, "lagos")),
        &Some(1),
        &Some(2),
    );
    assert!(!client.is_attester(&future_attester));
    assert_eq!(
        client.get_attester_status(&future_attester).unwrap().status,
        AttesterStatusKind::NotYetValid
    );

    let active_attester = Address::generate(&env);
    client.add_attester_with_info(&admin, &active_attester, &None, &None, &Some(0), &Some(1));
    assert!(client.is_attester(&active_attester));
    assert_eq!(
        client.get_attester_status(&active_attester).unwrap().status,
        AttesterStatusKind::Active
    );

    let expired_attester = Address::generate(&env);
    client.add_attester_with_info(&admin, &expired_attester, &None, &None, &None, &Some(0));
    assert!(!client.is_attester(&expired_attester));
    assert_eq!(
        client
            .get_attester_status(&expired_attester)
            .unwrap()
            .status,
        AttesterStatusKind::Expired
    );
}

#[test]
fn invalid_validity_window_is_rejected() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    assert_eq!(
        client.try_add_attester_with_info(&admin, &attester, &None, &None, &Some(10), &Some(10),),
        Err(Ok(Error::InvalidValidityWindow))
    );
    assert_eq!(client.get_attester_info(&attester), None);
}

#[test]
fn regional_registrar_is_limited_to_its_region_and_quota() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let registrar = Address::generate(&env);
    let lagos = Symbol::new(&env, "lagos");
    client.set_regional_registrar(&registrar, &lagos, &1);

    let first = Address::generate(&env);
    client.add_attester(&registrar, &first);
    assert!(client.is_attester(&first));
    assert_eq!(
        client.get_attester_info(&first).unwrap().region,
        Some(lagos.clone())
    );
    assert_eq!(client.get_regional_registrar_count(&registrar), 1);

    let second = Address::generate(&env);
    assert_eq!(
        client.try_add_attester(&registrar, &second),
        Err(Ok(Error::RegionalQuotaExceeded))
    );

    client.remove_attester(&admin, &first);
    assert_eq!(client.get_regional_registrar_count(&registrar), 0);
    client.add_attester(&registrar, &second);
    assert_eq!(client.get_regional_registrar_count(&registrar), 1);

    assert_eq!(
        client.try_set_regional_registrar(&registrar, &Symbol::new(&env, "abuja"), &2),
        Err(Ok(Error::RegionalAttestersRemain))
    );
    assert_eq!(
        client.get_regional_registrar(&registrar).unwrap().region,
        lagos
    );

    client.remove_attester(&admin, &second);
    client.set_regional_registrar(&registrar, &Symbol::new(&env, "abuja"), &2);
    assert_eq!(
        client.get_regional_registrar(&registrar).unwrap().region,
        Symbol::new(&env, "abuja")
    );

    let third = Address::generate(&env);
    let fourth = Address::generate(&env);
    client.add_attesters(
        &registrar,
        &Vec::from_array(&env, [third.clone(), fourth.clone()]),
    );
    assert_eq!(client.get_regional_registrar_count(&registrar), 2);
    client.remove_attesters(&admin, &Vec::from_array(&env, [third, fourth]));
    assert_eq!(client.get_regional_registrar_count(&registrar), 0);

    client.revoke_regional_registrar(&registrar);
    assert_eq!(client.get_regional_registrar(&registrar), None);
    assert_eq!(
        client.try_add_attester(&registrar, &Address::generate(&env)),
        Err(Ok(Error::RoleNotGranted))
    );
}

#[test]
fn regional_registrar_cannot_assign_a_different_region() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let registrar = Address::generate(&env);
    client.set_regional_registrar(&registrar, &Symbol::new(&env, "lagos"), &2);
    let attester = Address::generate(&env);

    assert_eq!(
        client.try_add_attester_with_info(
            &registrar,
            &attester,
            &None,
            &Some(Symbol::new(&env, "abuja")),
            &None,
            &None,
        ),
        Err(Ok(Error::RegionMismatch))
    );
    assert_eq!(client.get_attester_info(&attester), None);
    assert_eq!(client.get_regional_registrar_count(&registrar), 0);

    client.add_attester(&registrar, &attester);
    assert_eq!(
        client.get_attester_info(&attester).unwrap().region,
        Some(Symbol::new(&env, "lagos"))
    );
    assert_eq!(
        client.try_add_attesters(
            &registrar,
            &Vec::from_array(&env, [Address::generate(&env), Address::generate(&env)]),
        ),
        Err(Ok(Error::RegionalQuotaExceeded))
    );
    assert_eq!(client.get_regional_registrar_count(&registrar), 1);
}

#[test]
fn regional_registrar_can_suspend_only_its_region() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let registrar = Address::generate(&env);
    client.set_regional_registrar(&registrar, &Symbol::new(&env, "lagos"), &2);
    let lagos_attester = Address::generate(&env);
    let abuja_attester = Address::generate(&env);
    client.add_attester(&registrar, &lagos_attester);
    client.add_attester_with_info(
        &admin,
        &abuja_attester,
        &None,
        &Some(Symbol::new(&env, "abuja")),
        &None,
        &None,
    );

    client.suspend_attester(&registrar, &lagos_attester);
    assert!(!client.is_attester(&lagos_attester));
    assert_eq!(
        client.try_suspend_attester(&registrar, &abuja_attester),
        Err(Ok(Error::RegionMismatch))
    );
}

#[test]
fn validity_schema_migration_preserves_legacy_attester_records() {
    let (env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    let attester = Address::generate(&env);
    let license_hash = BytesN::from_array(&env, &[9u8; 32]);
    let region = Symbol::new(&env, "west");
    env.as_contract(&client.address, || {
        env.storage().persistent().set(
            &DataKey::Attester(attester.clone()),
            &StoredAttesterInfo {
                license_hash: Some(license_hash.clone()),
                region: Some(region.clone()),
            },
        );
        env.storage().instance().set(&DataKey::SchemaVersion, &3u32);
    });

    client.migrate();

    assert_eq!(client.get_schema_version(), 4);
    assert!(client.is_attester(&attester));
    assert_eq!(
        client.get_attester_info(&attester),
        Some(AttesterInfo {
            license_hash: Some(license_hash),
            region: Some(region),
            valid_from: None,
            valid_until: None,
        })
    );
}

#[test]
fn admin_address_can_be_added_as_attester() {
    let (_env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    // The admin's own address IS permitted as an attester — no special-case rejection exists.
    client.add_attester(&admin, &admin);
    assert!(client.is_attester(&admin));
}

#[test]
fn contract_address_can_be_added_as_attester() {
    let (_env, client, admin) = setup();
    initialize_for_tests(&client, &admin);

    // The contract's own address IS permitted as an attester — no special-case rejection exists.
    client.add_attester(&admin, &client.address);
    assert!(client.is_attester(&client.address));
}

#[test]
fn second_propose_admin_call_overwrites_pending_proposal() {
    let env = Env::default();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let address1 = Address::generate(&env);
    let address2 = Address::generate(&env);

    env.mock_all_auths();
    initialize_for_tests(&client, &admin);

    client.propose_admin(&address1);
    client.propose_admin(&address2);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &address1,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_accept_admin();
    assert!(result.is_err());

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &address2,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_accept_admin();
    assert_eq!(result, Ok(Ok(())));
}
