extern crate std;

use crate::test::{register_account, signatures_for, signing_keys};
use crate::{
    ContextPolicy, MultisigAccount, MultisigAccountClient, SignerConfig, SignerKey, SignerPolicy,
};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::Address as _,
    xdr::{
        Hash as XdrHash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs,
        Limits, ScVal, SorobanAddressCredentials, SorobanAuthorizationEntry,
        SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials, WriteXdr,
    },
    Address, Env, IntoVal, Symbol, Val, Vec,
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

fn contract_creation_invocation(env: &Env, contract: &Address) -> SorobanAuthorizedInvocation {
    SorobanAuthorizedInvocation {
        function: SorobanAuthorizedFunction::CreateContractHostFn(
            soroban_sdk::xdr::CreateContractArgs {
                contract_id_preimage: soroban_sdk::xdr::ContractIdPreimage::Address(
                    soroban_sdk::xdr::ContractIdPreimageFromAddress {
                        address: contract.into(),
                        salt: soroban_sdk::xdr::Uint256([7u8; 32]),
                    },
                ),
                executable: soroban_sdk::xdr::ContractExecutable::Wasm(XdrHash([0u8; 32])),
            },
        ),
        sub_invocations: Default::default(),
    }
}

fn policy(threshold: u32) -> SignerPolicy {
    SignerPolicy {
        threshold,
        signers: Vec::new(&Env::default()),
    }
}

fn context_policy(
    env: &Env,
    contract: &Address,
    function: &str,
    threshold: u32,
) -> ContextPolicy {
    ContextPolicy {
        context: (contract.clone(), Symbol::new(env, function)),
        policy: policy(threshold),
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

#[test]
fn strictest_context_policy_applies_to_multi_context_authorizations() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account.clone(),));
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    // Routine operation: one signature is enough.
    client.set_context_policy(
        &context_policy(&env, &attester_registry_id, "add_attester", 1),
        &keys[..2],
    );

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
        &keys[..1],
        10,
    );
    attester_registry
        .set_auths(&[add_attester])
        .add_attester(&account, &attester);
    assert!(attester_registry.is_attester(&attester));

    // Multi-context authorization: the strictest policy (default threshold 2) wins.
    let mut multi = invocation(
        &attester_registry_id,
        "add_attester",
        Vec::from_array(
            &env,
            [
                account.clone().into_val(&env),
                attester.clone().into_val(&env),
            ],
        ),
    );
    multi.sub_invocations = Vec::from_array(
        &env,
        [invocation(
            &attester_registry_id,
            "remove_attester",
            Vec::from_array(
                &env,
                [
                    account.clone().into_val(&env),
                    attester.clone().into_val(&env),
                ],
            ),
        )],
    );

    let under_signed = authorization_entry(&env, &account, multi.clone(), &keys[..1], 11);
    assert!(attester_registry
        .set_auths(&[under_signed])
        .try_remove_attester(&account, &attester)
        .is_err());
    assert!(attester_registry.is_attester(&attester));

    let fully_signed = authorization_entry(&env, &account, multi, &keys[..2], 12);
    attester_registry
        .set_auths(&[fully_signed])
        .remove_attester(&account, &attester);
    assert!(!attester_registry.is_attester(&attester));
}

#[test]
fn contract_creation_context_falls_back_to_strictest_policy() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account.clone(),));
    client.set_context_policy(
        &context_policy(&env, &attester_registry_id, "add_attester", 1),
        &keys[..2],
    );

    let creation = contract_creation_invocation(&env, &account);
    let under_signed = authorization_entry(&env, &account, creation.clone(), &keys[..1], 20);
    assert!(client
        .set_auths(&[under_signed])
        .try_set_context_policy(
            &context_policy(&env, &attester_registry_id, "remove_attester", 1),
            &keys[..2],
        )
        .is_err());

    let fully_signed = authorization_entry(&env, &account, creation, &keys[..2], 21);
    client
        .set_auths(&[fully_signed])
        .set_context_policy(
            &context_policy(&env, &attester_registry_id, "remove_attester", 1),
            &keys[..2],
        );
}

#[test]
fn replace_signer_then_admin_call_with_new_key() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account.clone(),));
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

    let old_key = SignerKey::Ed25519(keys[0].verifying_key().to_bytes());
    let new_signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let new_key = SignerKey::Ed25519(new_signing_key.verifying_key().to_bytes());

    client.replace_signer(
        &old_key,
        &SignerConfig {
            key: new_key.clone(),
            weight: 1,
            roles: Vec::new(&env),
        },
        &keys[..2],
    );

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
        &[keys[1].clone(), new_signing_key],
        30,
    );
    attester_registry
        .set_auths(&[add_attester])
        .add_attester(&account, &attester);
    assert!(attester_registry.is_attester(&attester));
}
