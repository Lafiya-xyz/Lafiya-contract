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
fn get_schema_version_succeeds() {
    // Asserts the literal current schema version, not just that the call
    // succeeds. Any change to this expected value is a schema version bump
    // and must be deliberate, paired with a migration plan (see
    // `needs_migration`/`migrate` in lib.rs), and not an accidental side
    // effect of an unrelated change.
    let (_, client, admin) = setup();
    assert_eq!(client.get_schema_version(), 1);
    client.initialize(&admin);
    assert_eq!(client.get_schema_version(), 1);
}

#[test]
fn initialize_sets_admin() {
    let (_, client, admin) = setup();
    client.initialize(&admin);
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
    client.initialize(&admin);

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
    client.initialize(&admin);
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
    client.initialize(&admin);

    let result = client.try_accept_admin();
    assert_eq!(result, Err(Ok(Error::NoPendingTransfer)));
}

#[test]
fn successful_admin_transfer_flow() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

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

    let attester = Address::generate(&env);
    client.add_attester(&attester);

    assert_eq!(
        env.auths(),
        std::vec![(
            new_admin.clone(),
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

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_attester",
            args: (attester.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_add_attester(&attester);
    assert!(result.is_err());
}

#[test]
fn add_attester_beyond_cap_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);
    client.set_max_attesters(&2);

    client.add_attester(&Address::generate(&env));
    client.add_attester(&Address::generate(&env));
    assert_eq!(client.get_attester_count(), 2);

    let result = client.try_add_attester(&Address::generate(&env));
    assert_eq!(result, Err(Ok(Error::AllowlistFull)));
    assert_eq!(client.get_attester_count(), 2);
}

#[test]
fn removing_an_attester_frees_cap_slot() {
    let (env, client, admin) = setup();
    client.initialize(&admin);
    client.set_max_attesters(&1);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    assert_eq!(
        client.try_add_attester(&Address::generate(&env)),
        Err(Ok(Error::AllowlistFull))
    );

    client.remove_attester(&attester);
    assert_eq!(client.get_attester_count(), 0);
    client.add_attester(&Address::generate(&env));
    assert_eq!(client.get_attester_count(), 1);
}

#[test]
fn re_adding_an_existing_attester_does_not_consume_cap() {
    let (env, client, admin) = setup();
    client.initialize(&admin);
    client.set_max_attesters(&1);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    client.add_attester(&attester);
    assert_eq!(client.get_attester_count(), 1);
}

#[test]
fn update_attester_info_on_unknown_attester_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    let license_hash = BytesN::from_array(&env, &[1u8; 32]);
    let region = Symbol::new(&env, "west");
    let result = client.try_update_attester_info(&attester, &Some(license_hash), &Some(region));
    assert_eq!(result, Err(Ok(Error::AttesterNotFound)));
}

#[test]
fn update_attester_info_on_removed_attester_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    client.remove_attester(&attester);

    let result = client.try_update_attester_info(&attester, &None, &None);
    assert_eq!(result, Err(Ok(Error::AttesterNotFound)));
}

#[test]
fn update_attester_info_updates_metadata_and_emits_distinct_event() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    let initial_hash = BytesN::from_array(&env, &[1u8; 32]);
    let initial_region = Symbol::new(&env, "west");
    client.add_attester_with_info(&attester, &Some(initial_hash), &Some(initial_region));

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
        &attester,
        &Some(updated_hash.clone()),
        &Some(updated_region.clone()),
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
    client.initialize(&admin);
    client.add_attester(&attester);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attester,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "update_attester_info",
            args: (attester.clone(), None::<BytesN<32>>, None::<Symbol>).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_update_attester_info(&attester, &None, &None);
    assert_eq!(result, Err(Err(soroban_sdk::InvokeError::Abort)));
}

#[test]
fn update_attester_info_while_paused_fails() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    client.pause();

    let result = client.try_update_attester_info(&attester, &None, &None);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn get_attester_status_for_unknown_attester_is_none() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    assert_eq!(client.get_attester_status(&attester), None);
}

#[test]
fn get_attester_status_for_removed_attester_is_none() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    let attester = Address::generate(&env);
    client.add_attester(&attester);
    client.remove_attester(&attester);

    assert_eq!(client.get_attester_status(&attester), None);
}

#[test]
fn lowering_max_attesters_below_current_count_does_not_evict() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    // Add 3 attesters with no cap restriction.
    let attester1 = Address::generate(&env);
    let attester2 = Address::generate(&env);
    let attester3 = Address::generate(&env);
    client.add_attester(&attester1);
    client.add_attester(&attester2);
    client.add_attester(&attester3);
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
    let result = client.try_add_attester(&new_attester);
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
    client.initialize(&admin);

    let never_added = Address::generate(&env);

    // Precondition: the address has never been allowlisted.
    assert!(!client.is_attester(&never_added));

    // suspend_attester succeeds (no error) even though the address was never added.
    client.suspend_attester(&never_added);

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
    client.initialize(&admin);

    let attester = Address::generate(&env);
    let license_hash = BytesN::from_array(&env, &[3u8; 32]);
    let region = Symbol::new(&env, "north");
    client.add_attester_with_info(
        &attester,
        &Some(license_hash.clone()),
        &Some(region.clone()),
    );

    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash.clone()),
                region: Some(region.clone()),
            },
            suspended: false,
        }),
    );
    assert!(client.is_attester(&attester));

    client.suspend_attester(&attester);
    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash.clone()),
                region: Some(region.clone()),
            },
            suspended: true,
        }),
    );
    assert!(!client.is_attester(&attester));

    client.reinstate_attester(&attester);
    assert_eq!(
        client.get_attester_status(&attester),
        Some(AttesterStatus {
            info: AttesterInfo {
                license_hash: Some(license_hash),
                region: Some(region),
            },
            suspended: false,
        }),
    );
    assert!(client.is_attester(&attester));
}

#[test]
fn admin_address_can_be_added_as_attester() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    // The admin's own address IS permitted as an attester — no special-case rejection exists.
    client.add_attester(&admin);
    assert!(client.is_attester(&admin));
}

#[test]
fn contract_address_can_be_added_as_attester() {
    let (env, client, admin) = setup();
    client.initialize(&admin);

    // The contract's own address IS permitted as an attester — no special-case rejection exists.
    client.add_attester(&client.address);
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
    client.initialize(&admin);

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
    assert_eq!(result, Ok(()));
}
