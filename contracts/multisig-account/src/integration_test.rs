extern crate std;

use crate::test::{register_account, signatures_for, signing_keys};
use crate::{Signer, SignerPolicy};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    xdr::{
        Hash as XdrHash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs,
        Limits, ScVal, SorobanAddressCredentials, SorobanAuthorizationEntry,
        SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials, WriteXdr,
    },
    Address, BytesN, Env, IntoVal, Val, Vec,
};

fn invocation(contract: &Address, function: &str, args: Vec<Val>) -> SorobanAuthorizedInvocation {
    SorobanAuthorizedInvocation {
        function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
            contract_address: contract.into(),
            function_name: function.try_into().unwrap(),
            args: args.into(),
        }),
        sub_invocations: Default::default(),
    }
}

fn authorization_entry(
    env: &Env,
    account: &Address,
    invocation: SorobanAuthorizedInvocation,
    keys: &[SigningKey],
    nonce: i64,
) -> SorobanAuthorizationEntry {
    let expiration = env.ledger().sequence() + 100;
    let preimage = HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
        network_id: XdrHash(env.ledger().network_id().to_array()),
        nonce,
        signature_expiration_ledger: expiration,
        invocation: invocation.clone(),
    });
    let payload: [u8; 32] = Sha256::digest(preimage.to_xdr(Limits::none()).unwrap()).into();
    let signatures = signatures_for(env, keys, &payload);

    SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address: account.into(),
            nonce,
            signature_expiration_ledger: expiration,
            signature: ScVal::from(signatures),
        }),
        root_invocation: invocation,
    }
}

#[test]
fn multisig_address_administers_both_registries() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);

    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account.clone(),));
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let attestation_registry_id = env.register(
        attestation_registry::AttestationRegistry,
        (account.clone(), attester_registry_id.clone()),
    );
    let _attestation_registry =
        attestation_registry::AttestationRegistryClient::new(&env, &attestation_registry_id);

    let grant_registrar = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "grant_role",
            Vec::from_array(
                &env,
                [
                    attester_registry::Role::Registrar.into_val(&env),
                    account.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..2],
        3,
    );
    attester_registry
        .set_auths(&[grant_registrar])
        .grant_role(&attester_registry::Role::Registrar, &account);

    let attester = Address::generate(&env);
    let add_attester = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "add_attester",
            Vec::from_array(
                &env,
                [
                    account.clone().into_val(&env),
                    attester.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..2],
        4,
    );
    attester_registry
        .set_auths(&[add_attester])
        .add_attester(&account, &attester);
    assert!(attester_registry.is_attester(&attester));

    let unauthorized_remove = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "remove_attester",
            Vec::from_array(
                &env,
                [
                    account.clone().into_val(&env),
                    attester.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..1],
        5,
    );
    assert!(attester_registry
        .set_auths(&[unauthorized_remove])
        .try_remove_attester(&account, &attester)
        .is_err());
    assert!(attester_registry.is_attester(&attester));

    let remove_attester = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "remove_attester",
            Vec::from_array(
                &env,
                [
                    account.clone().into_val(&env),
                    attester.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..2],
        6,
    );
    attester_registry
        .set_auths(&[remove_attester])
        .remove_attester(&account, &attester);
    assert!(!attester_registry.is_attester(&attester));
}

fn policy_for(env: &Env, keys: &[SigningKey], threshold: u32) -> SignerPolicy {
    let mut signers = Vec::new(env);
    for key in keys {
        signers.push_back(Signer::Ed25519(BytesN::from_array(
            env,
            &key.verifying_key().to_bytes(),
        )));
    }
    SignerPolicy {
        signers,
        threshold,
    }
}

#[test]
fn policy_constructor_and_guarded_upgrade_administer_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let policy = policy_for(&env, &keys, 2);
    let account = crate::MultisigAccount::new_policy(&env, policy.clone());
    let account_address = Address::from_contract_id(&env, &account);

    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account_address.clone(),));
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    // The policy-based account can administer the registry through a real call.
    let grant_registrar = authorization_entry(
        &env,
        &account_address,
        invocation(
            &attester_registry_id,
            "grant_role",
            Vec::from_array(
                &env,
                [
                    attester_registry::Role::Registrar.into_val(&env),
                    account_address.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..2],
        7,
    );
    attester_registry
        .set_auths(&[grant_registrar])
        .grant_role(&attester_registry::Role::Registrar, &account_address);

    // Schedule a guarded self-upgrade; the announcement event must be emitted.
    let new_wasm = BytesN::from_array(&env, &[7u8; 32]);
    let events_before = env.events().all().len();
    let schedule = authorization_entry(
        &env,
        &account_address,
        invocation(
            &account_address,
            "schedule_upgrade",
            Vec::from_array(&env, [new_wasm.clone().into_val(&env)]),
        ),
        &keys[..2],
        8,
    );
    crate::MultisigAccountClient::new(&env, &account_address)
        .set_auths(&[schedule])
        .schedule_upgrade(&new_wasm);
    assert!(env.events().all().len() > events_before);
    assert_eq!(
        crate::MultisigAccountClient::new(&env, &account_address).pending_upgrade(),
        Some(new_wasm.clone())
    );

    // Executing before the delay elapses must fail.
    let early_execute = authorization_entry(
        &env,
        &account_address,
        invocation(&account_address, "execute_upgrade", Vec::new(&env)),
        &keys[..2],
        9,
    );
    assert!(crate::MultisigAccountClient::new(&env, &account_address)
        .set_auths(&[early_execute])
        .try_execute_upgrade()
        .is_err());

    // After the delay, the upgrade executes and clears the pending state.
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + crate::UPGRADE_DELAY_LEDGERS + 1);
    let execute = authorization_entry(
        &env,
        &account_address,
        invocation(&account_address, "execute_upgrade", Vec::new(&env)),
        &keys[..2],
        10,
    );
    crate::MultisigAccountClient::new(&env, &account_address)
        .set_auths(&[execute])
        .execute_upgrade();
    assert_eq!(
        crate::MultisigAccountClient::new(&env, &account_address).pending_upgrade(),
        None
    );
}
