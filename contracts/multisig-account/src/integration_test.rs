extern crate std;

use crate::test::{register_account, signatures_for, signing_keys};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::Address as _,
    xdr::{
        Hash as XdrHash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs,
        Limits, ScVal, SorobanAddressCredentials, SorobanAuthorizationEntry,
        SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials, WriteXdr,
    },
    Address, Env, IntoVal, Val, Vec,
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
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);

    let attester_registry_id = env.register(attester_registry::AttesterRegistry, ());
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);
    let initialize_attesters = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "initialize",
            Vec::from_array(&env, [account.clone().into_val(&env)]),
        ),
        &keys[..2],
        1,
    );
    attester_registry
        .set_auths(&[initialize_attesters])
        .initialize(&account);

    let attestation_registry_id = env.register(attestation_registry::AttestationRegistry, ());
    let attestation_registry =
        attestation_registry::AttestationRegistryClient::new(&env, &attestation_registry_id);
    let initialize_attestations = authorization_entry(
        &env,
        &account,
        invocation(
            &attestation_registry_id,
            "initialize",
            Vec::from_array(
                &env,
                [
                    account.clone().into_val(&env),
                    attester_registry_id.clone().into_val(&env),
                ],
            ),
        ),
        &keys[..2],
        2,
    );
    attestation_registry
        .set_auths(&[initialize_attestations])
        .initialize(&account, &attester_registry_id);
    assert_eq!(
        attestation_registry.try_initialize(&account, &attester_registry_id),
        Err(Ok(attestation_registry::Error::AlreadyInitialized))
    );

    let attester = Address::generate(&env);
    let add_attester = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "add_attester",
            Vec::from_array(&env, [attester.clone().into_val(&env)]),
        ),
        &keys[..2],
        3,
    );
    attester_registry
        .set_auths(&[add_attester])
        .add_attester(&attester);
    assert!(attester_registry.is_attester(&attester));

    let unauthorized_remove = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "remove_attester",
            Vec::from_array(&env, [attester.clone().into_val(&env)]),
        ),
        &keys[..1],
        4,
    );
    assert!(attester_registry
        .set_auths(&[unauthorized_remove])
        .try_remove_attester(&attester)
        .is_err());
    assert!(attester_registry.is_attester(&attester));

    let remove_attester = authorization_entry(
        &env,
        &account,
        invocation(
            &attester_registry_id,
            "remove_attester",
            Vec::from_array(&env, [attester.clone().into_val(&env)]),
        ),
        &keys[..2],
        5,
    );
    attester_registry
        .set_auths(&[remove_attester])
        .remove_attester(&attester);
    assert!(!attester_registry.is_attester(&attester));
}
