extern crate std;

use crate::test::{register_account, signatures_for, signing_keys};
use crate::{MultisigAccountClient, RoleRequirement, SignerPolicy};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::Address as _,
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

#[test]
fn unsatisfiable_policy_is_rejected_and_account_stays_usable() {
    let env = Env::default();
    env.mock_all_auths();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let signers = Vec::from_array(
        &env,
        [
            BytesN::from_array(&env, &keys[0].verifying_key().to_bytes()),
            BytesN::from_array(&env, &keys[1].verifying_key().to_bytes()),
        ],
    );

    // Weight threshold exceeds the sum of all signer weights: unsatisfiable.
    let unsatisfiable = SignerPolicy {
        signers: signers.clone(),
        weight_threshold: 100,
        minimum_signers: 1,
        role_requirements: Vec::new(&env),
        allow_single_point_of_failure: false,
    };
    assert!(client.try_set_policy(&unsatisfiable).is_err());

    // Role requirement demands more signers of a role than exist: unsatisfiable.
    let over_role = SignerPolicy {
        signers: signers.clone(),
        weight_threshold: 1,
        minimum_signers: 1,
        role_requirements: Vec::from_array(
            &env,
            [RoleRequirement {
                role: 0,
                minimum: 5,
            }],
        ),
        allow_single_point_of_failure: false,
    };
    assert!(client.try_set_policy(&over_role).is_err());

    // The account is not bricked: the original policy still governs and a
    // valid policy can still be installed afterwards.
    let valid = SignerPolicy {
        signers: signers.clone(),
        weight_threshold: 2,
        minimum_signers: 2,
        role_requirements: Vec::new(&env),
        allow_single_point_of_failure: false,
    };
    client.set_policy(&valid);

    // The account still administers the registries under the new policy.
    let attester_registry_id =
        env.register(attester_registry::AttesterRegistry, (account.clone(),));
    let attester_registry =
        attester_registry::AttesterRegistryClient::new(&env, &attester_registry_id);

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
        7,
    );
    attester_registry
        .set_auths(&[grant_registrar])
        .grant_role(&attester_registry::Role::Registrar, &account);
}
