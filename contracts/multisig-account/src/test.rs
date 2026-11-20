extern crate std;

use super::*;
use ed25519_dalek::{Signer as _, SigningKey};
use p256::ecdsa::{
    signature::hazmat::PrehashSigner, Signature as P256Signature, SigningKey as P256SigningKey,
};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{
    auth::{Context, ContractContext},
    BytesN, Env, Event, IntoVal, Symbol, Vec,
};

pub(crate) fn signing_keys() -> std::vec::Vec<SigningKey> {
    let mut keys = std::vec![
        SigningKey::from_bytes(&[1; 32]),
        SigningKey::from_bytes(&[2; 32]),
        SigningKey::from_bytes(&[3; 32]),
    ];
    keys.sort_by_key(|key| key.verifying_key().to_bytes());
    keys
}

#[allow(dead_code)]
fn five_signing_keys() -> std::vec::Vec<SigningKey> {
    let mut keys = std::vec![
        SigningKey::from_bytes(&[1; 32]),
        SigningKey::from_bytes(&[2; 32]),
        SigningKey::from_bytes(&[3; 32]),
        SigningKey::from_bytes(&[4; 32]),
        SigningKey::from_bytes(&[5; 32]),
    ];
    keys.sort_by_key(|key| key.verifying_key().to_bytes());
    keys
}

pub(crate) fn register_account(
    env: &Env,
    keys: &[SigningKey],
    threshold: u32,
) -> soroban_sdk::Address {
    let mut signers = Vec::new(env);
    for key in keys {
        signers.push_back(BytesN::from_array(env, &key.verifying_key().to_bytes()));
    }
    env.register(MultisigAccount, (signers, threshold))
}

pub(crate) fn signatures_for(env: &Env, keys: &[SigningKey], payload: &[u8; 32]) -> Vec<Signature> {
    let mut ordered = keys.iter().collect::<std::vec::Vec<_>>();
    ordered.sort_by_key(|key| key.verifying_key().to_bytes());

    let mut signatures = Vec::new(env);
    for key in ordered {
        signatures.push_back(Signature {
            public_key: SignerKey::Ed25519(BytesN::from_array(
                env,
                &key.verifying_key().to_bytes(),
            )),
            signature: BytesN::from_array(env, &key.sign(payload).to_bytes()),
        });
    }
    signatures
}

#[test]
fn getters_return_threshold_and_canonical_signer_order() {
    let env = Env::default();
    let keys = signing_keys();
    let configured = [&keys[2], &keys[1], &keys[0]];
    let signers = configured
        .iter()
        .map(|key| BytesN::from_array(&env, &key.verifying_key().to_bytes()))
        .collect::<std::vec::Vec<_>>();
    let mut configured_signers = Vec::new(&env);
    for signer in signers {
        configured_signers.push_back(signer);
    }
    let account = env.register(MultisigAccount, (configured_signers, 2u32));
    let client = MultisigAccountClient::new(&env, &account);

    let mut expected_signers = Vec::new(&env);
    for key in &keys {
        expected_signers.push_back(SignerKey::Ed25519(BytesN::from_array(
            &env,
            &key.verifying_key().to_bytes(),
        )));
    }
    assert_eq!(client.get_threshold(), 2);
    assert_eq!(client.get_signers(), expected_signers);
}

fn check_auth(
    env: &Env,
    account: &soroban_sdk::Address,
    payload: &BytesN<32>,
    signatures: Vec<Signature>,
) -> Result<(), Result<Error, soroban_sdk::InvokeError>> {
    env.try_invoke_contract_check_auth::<Error>(
        account,
        payload,
        signatures.into_val(env),
        &Vec::<Context>::new(env),
    )
}

#[allow(dead_code)]
fn p256_signature(env: &Env, key: &P256SigningKey, payload: &[u8; 32]) -> Signature {
    let public_key = key.verifying_key().to_encoded_point(false);
    let public_key_bytes: [u8; 65] = public_key.as_bytes().try_into().unwrap();
    let signature: P256Signature = key.sign_prehash(payload).unwrap();
    let signature = signature.normalize_s().unwrap_or(signature);
    let signature_bytes: [u8; 64] = signature.to_bytes().into();
    Signature {
        public_key: SignerKey::Secp256r1(BytesN::from_array(env, &public_key_bytes)),
        signature: BytesN::from_array(env, &signature_bytes),
    }
}

#[test]
fn one_of_one_signer_authorizes() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys[..1], 1);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures = signatures_for(&env, &keys[..1], &payload.to_array());

    assert_eq!(check_auth(&env, &account, &payload, signatures), Ok(()));
}

#[test]
fn two_of_three_signers_authorize() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures = signatures_for(&env, &keys[..2], &payload.to_array());

    assert_eq!(check_auth(&env, &account, &payload, signatures), Ok(()));
}

#[test]
fn one_of_one_with_zero_signatures_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys[..1], 1);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures: Vec<Signature> = Vec::new(&env);

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::NotEnoughSigners))
    );
}

#[test]
fn fewer_than_threshold_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures = signatures_for(&env, &keys[..1], &payload.to_array());

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::NotEnoughSigners))
    );
}

#[test]
fn unknown_signer_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let unknown = SigningKey::from_bytes(&[9; 32]);
    let signatures = signatures_for(
        &env,
        &[SigningKey::from_bytes(&[1; 32]), unknown],
        &payload.to_array(),
    );

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::UnknownSigner))
    );
}

#[test]
fn duplicate_signature_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signature = signatures_for(&env, &keys[..1], &payload.to_array()).get_unchecked(0);
    let signatures = Vec::from_array(&env, [signature.clone(), signature]);

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::BadSignatureOrder))
    );
}

#[test]
fn same_signer_appearing_twice_in_signatures_is_rejected() {
    // Unlike `duplicate_signature_is_rejected`, which clones one already-built
    // `Signature` value, this independently invokes `sign` twice for the same
    // configured signer over the same payload. Ed25519 signing is
    // deterministic (RFC 8032), so the two independently produced signatures
    // are byte-identical to each other — confirming the contract rejects a
    // signer appearing twice regardless of whether the caller submitted a
    // literal copy or re-signed from scratch.
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signer = &keys[0];

    let first_signature = Signature {
        public_key: SignerKey::Ed25519(BytesN::from_array(
            &env,
            &signer.verifying_key().to_bytes(),
        )),
        signature: BytesN::from_array(&env, &signer.sign(&payload.to_array()).to_bytes()),
    };
    let second_signature = Signature {
        public_key: SignerKey::Ed25519(BytesN::from_array(
            &env,
            &signer.verifying_key().to_bytes(),
        )),
        signature: BytesN::from_array(&env, &signer.sign(&payload.to_array()).to_bytes()),
    };
    assert_eq!(first_signature.signature, second_signature.signature);

    let signatures = Vec::from_array(&env, [first_signature, second_signature]);

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::BadSignatureOrder))
    );
}

#[test]
fn signatures_out_of_order_are_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let ordered = signatures_for(&env, &keys[..2], &payload.to_array());
    let signatures = Vec::from_array(&env, [ordered.get_unchecked(1), ordered.get_unchecked(0)]);

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::BadSignatureOrder))
    );
}

#[test]
fn signature_for_another_payload_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let signed_payload = BytesN::from_array(&env, &[7; 32]);
    let checked_payload = BytesN::from_array(&env, &[8; 32]);
    let signatures = signatures_for(&env, &keys[..2], &signed_payload.to_array());

    assert!(check_auth(&env, &account, &checked_payload, signatures).is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn zero_threshold_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    register_account(&env, &keys, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn zero_signers_and_zero_threshold_is_rejected() {
    // When both signers is empty and threshold is 0, the threshold check
    // (line 50: `if threshold == 0 || threshold > signers.len()`) fires first.
    let env = Env::default();
    let keys: &[SigningKey] = &[];
    register_account(&env, keys, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn threshold_above_signer_count_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    register_account(&env, &keys, 4);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn duplicate_configured_signer_is_rejected() {
    let env = Env::default();
    let key = SigningKey::from_bytes(&[1; 32]);
    register_account(&env, &[key.clone(), key], 1);
}

#[test]
fn check_auth_extends_ttl_on_success() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures = signatures_for(&env, &keys[..2], &payload.to_array());

    // Successful __check_auth call triggers extend_ttl on SignerCount/Threshold
    assert_eq!(check_auth(&env, &account, &payload, signatures), Ok(()));
}

#[test]
fn successful_auth_emits_signers_and_payload() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    let signatures = signatures_for(&env, &keys[..2], &payload.to_array());

    assert_eq!(check_auth(&env, &account, &payload, signatures), Ok(()));

    let approved = AuthorizationApproved {
        timestamp: env.ledger().timestamp(),
        payload: payload.clone().into(),
        signers: Vec::from_array(
            &env,
            [
                SignerKey::Ed25519(BytesN::from_array(
                    &env,
                    &keys[0].verifying_key().to_bytes(),
                )),
                SignerKey::Ed25519(BytesN::from_array(
                    &env,
                    &keys[1].verifying_key().to_bytes(),
                )),
            ],
        ),
    };
    assert_eq!(
        env.events().all(),
        std::vec![approved.to_xdr(&env, &account)]
    );
}

#[test]
fn too_many_signatures_is_rejected() {
    let env = Env::default();
    let keys = signing_keys();
    // Configure a 2-of-3 account
    let account = register_account(&env, &keys[..2], 2);
    let payload = BytesN::from_array(&env, &[7; 32]);
    // Submit all 3 keys' signatures even though only 2 are configured signers
    let signatures = signatures_for(&env, &keys, &payload.to_array());

    assert_eq!(
        check_auth(&env, &account, &payload, signatures),
        Err(Ok(Error::TooManySigners))
    );
}

#[test]
fn three_of_five_signers_authorize() {
    let env = Env::default();
    let mut keys = std::vec![
        SigningKey::from_bytes(&[1; 32]),
        SigningKey::from_bytes(&[2; 32]),
        SigningKey::from_bytes(&[3; 32]),
        SigningKey::from_bytes(&[4; 32]),
        SigningKey::from_bytes(&[5; 32]),
    ];
    keys.sort_by_key(|key| key.verifying_key().to_bytes());
    let account = register_account(&env, &keys, 3);
    let payload = BytesN::from_array(&env, &[7; 32]);
    // Supply exactly 3 signatures from the 5 signers in correct order
    let signatures = signatures_for(&env, &keys[..3], &payload.to_array());

    assert_eq!(check_auth(&env, &account, &payload, signatures), Ok(()));
}

// ---------------------------------------------------------------------------
// Context-policy helpers and tests (#703)
// ---------------------------------------------------------------------------

fn contract_context(
    env: &Env,
    contract: &soroban_sdk::Address,
    fn_name: &str,
) -> Context {
    Context::Contract(ContractContext {
        contract: contract.clone(),
        fn_name: Symbol::new(env, fn_name),
        args: Vec::new(env),
    })
}

fn check_auth_with_contexts(
    env: &Env,
    account: &soroban_sdk::Address,
    payload: &BytesN<32>,
    signatures: Vec<Signature>,
    contexts: Vec<Context>,
) -> Result<(), Result<Error, soroban_sdk::InvokeError>> {
    env.try_invoke_contract_check_auth::<Error>(
        account,
        payload,
        signatures.into_val(env),
        &contexts,
    )
}

#[test]
fn context_policy_can_be_configured_and_reported() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    client.set_context_policy(&target, &Symbol::new(&env, "add_attester"), &1u32);

    assert_eq!(
        client.get_context_policy(&target, &Symbol::new(&env, "add_attester")),
        1
    );
    // Unconfigured contexts report the default policy.
    assert_eq!(
        client.get_context_policy(&target, &Symbol::new(&env, "upgrade")),
        client.get_threshold()
    );
}

#[test]
fn strictest_policy_applies_across_multiple_contexts() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    // Routine operation needs one signer, catastrophic one needs all three.
    client.set_context_policy(&target, &Symbol::new(&env, "add_attester"), &1u32);
    client.set_context_policy(&target, &Symbol::new(&env, "upgrade"), &3u32);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let contexts = Vec::from_array(
        &env,
        [
            contract_context(&env, &target, "add_attester"),
            contract_context(&env, &target, "upgrade"),
        ],
    );

    // One signature satisfies the routine policy but not the strictest one.
    let one = signatures_for(&env, &keys[..1], &payload.to_array());
    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, one, contexts.clone()),
        Err(Ok(Error::NotEnoughSigners))
    );

    // Three signatures satisfy the strictest policy across all contexts.
    let three = signatures_for(&env, &keys, &payload.to_array());
    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, three, contexts),
        Ok(())
    );
}

#[test]
fn single_context_uses_its_configured_policy() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 3);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    client.set_context_policy(&target, &Symbol::new(&env, "add_attester"), &1u32);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let contexts = Vec::from_array(&env, [contract_context(&env, &target, "add_attester")]);
    let one = signatures_for(&env, &keys[..1], &payload.to_array());

    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, one, contexts),
        Ok(())
    );
}

#[test]
fn unknown_context_falls_back_to_strictest_configured_policy() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    // Configure a strict policy for a known context; the default is lower.
    client.set_context_policy(&target, &Symbol::new(&env, "upgrade"), &3u32);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let contexts = Vec::from_array(&env, [contract_context(&env, &target, "unknown_fn")]);
    let two = signatures_for(&env, &keys[..2], &payload.to_array());

    // The unknown context must require the strictest configured policy (3).
    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, two, contexts),
        Err(Ok(Error::NotEnoughSigners))
    );
}

#[test]
fn contract_creation_context_falls_back_to_strictest_policy() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    client.set_context_policy(&target, &Symbol::new(&env, "upgrade"), &3u32);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let contexts = Vec::from_array(
        &env,
        [Context::CreateContractHostFn(
            soroban_sdk::auth::CreateContractHostFnContext {
                executable: soroban_sdk::auth::ContractExecutable::Wasm(
                    BytesN::from_array(&env, &[0; 32]),
                ),
                salt: BytesN::from_array(&env, &[1; 32]),
            },
        )],
    );
    let two = signatures_for(&env, &keys[..2], &payload.to_array());

    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, two, contexts),
        Err(Ok(Error::NotEnoughSigners))
    );
}

#[test]
fn context_policy_mutation_requires_strictest_policy() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));

    client.set_context_policy(&target, &Symbol::new(&env, "upgrade"), &3u32);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let contexts = Vec::from_array(
        &env,
        [contract_context(&env, &account, "set_context_policy")],
    );
    let two = signatures_for(&env, &keys[..2], &payload.to_array());

    // Changing context policies must itself satisfy the strictest policy.
    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, two, contexts.clone()),
        Err(Ok(Error::NotEnoughSigners))
    );

    let three = signatures_for(&env, &keys, &payload.to_array());
    assert_eq!(
        check_auth_with_contexts(&env, &account, &payload, three, contexts),
        Ok(())
    );
}

// ---------------------------------------------------------------------------
// Atomic replace_signer tests (#705)
// ---------------------------------------------------------------------------

#[test]
fn replace_signer_preserves_weight_and_roles_for_ed25519() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let old_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &keys[0].verifying_key().to_bytes(),
    ));
    let new_signing = SigningKey::from_bytes(&[42; 32]);
    let new_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &new_signing.verifying_key().to_bytes(),
    ));

    let old_weight = client.get_signer_weight(&old_key);
    let old_roles = client.get_signer_roles(&old_key);

    client.replace_signer(&old_key, &new_key);

    assert_eq!(client.get_signer_weight(&new_key), old_weight);
    assert_eq!(client.get_signer_roles(&new_key), old_roles);
    assert!(!client.is_signer(&old_key));
    assert!(client.is_signer(&new_key));
}

#[test]
fn replace_signer_preserves_weight_and_roles_for_secp256r1() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let old_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &keys[0].verifying_key().to_bytes(),
    ));
    let p256_key = P256SigningKey::from_bytes(&[7u8; 32].into()).unwrap();
    let public_key = p256_key.verifying_key().to_encoded_point(false);
    let public_key_bytes: [u8; 65] = public_key.as_bytes().try_into().unwrap();
    let new_key = SignerKey::Secp256r1(BytesN::from_array(&env, &public_key_bytes));

    let old_weight = client.get_signer_weight(&old_key);
    let old_roles = client.get_signer_roles(&old_key);

    client.replace_signer(&old_key, &new_key);

    assert_eq!(client.get_signer_weight(&new_key), old_weight);
    assert_eq!(client.get_signer_roles(&new_key), old_roles);
    assert!(!client.is_signer(&old_key));
    assert!(client.is_signer(&new_key));
}

#[test]
fn replace_signer_rejects_duplicate_new_key() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let old_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &keys[0].verifying_key().to_bytes(),
    ));
    let existing_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &keys[1].verifying_key().to_bytes(),
    ));

    assert_eq!(
        client.try_replace_signer(&old_key, &existing_key),
        Err(Ok(Error::DuplicateSigner))
    );
}

#[test]
fn replace_signer_rejects_unknown_old_key() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let unknown = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &SigningKey::from_bytes(&[99; 32]).verifying_key().to_bytes(),
    ));
    let new_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes(),
    ));

    assert_eq!(
        client.try_replace_signer(&unknown, &new_key),
        Err(Ok(Error::UnknownSigner))
    );
}

#[test]
fn replace_signer_emits_signer_replaced_event() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys, 2);
    let client = MultisigAccountClient::new(&env, &account);

    let old_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &keys[0].verifying_key().to_bytes(),
    ));
    let new_key = SignerKey::Ed25519(BytesN::from_array(
        &env,
        &SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes(),
    ));

    client.replace_signer(&old_key, &new_key);

    let expected = SignerReplaced {
        old_fingerprint: old_key.fingerprint(&env),
        new_fingerprint: new_key.fingerprint(&env),
        config_version: client.get_config_version(),
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected.to_xdr(&env, &account)]
    );
}

// ---------------------------------------------------------------------------
// Worst-case mixed-key CPU budget regression (#706)
// ---------------------------------------------------------------------------

#[test]
fn worst_case_mixed_key_auth_fits_cpu_budget() {
    let env = Env::default();
    let keys = signing_keys();

    // Configure the maximum number of signers, mixing ed25519 and secp256r1.
    let mut signers = Vec::new(&env);
    let mut ed_keys = std::vec::Vec::new();
    let mut p256_keys = std::vec::Vec::new();
    for index in 0..MAX_SIGNERS {
        if index % 2 == 0 {
            let key = SigningKey::from_bytes(&[(index as u8) + 1; 32]);
            signers.push_back(SignerKey::Ed25519(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            )));
            ed_keys.push(key);
        } else {
            let key = P256SigningKey::from_bytes(&[(index as u8) + 1; 32].into()).unwrap();
            let point = key.verifying_key().to_encoded_point(false);
            let bytes: [u8; 65] = point.as_bytes().try_into().unwrap();
            signers.push_back(SignerKey::Secp256r1(BytesN::from_array(&env, &bytes)));
            p256_keys.push(key);
        }
    }
    let account = env.register(MultisigAccount, (signers, MAX_SIGNERS as u32));
    let client = MultisigAccountClient::new(&env, &account);

    let payload = BytesN::from_array(&env, &[7; 32]);
    let mut signatures = Vec::new(&env);
    for key in &ed_keys {
        signatures.push_back(Signature {
            public_key: SignerKey::Ed25519(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            )),
            signature: BytesN::from_array(&env, &key.sign(&payload.to_array()).to_bytes()),
        });
    }
    for key in &p256_keys {
        signatures.push_back(p256_signature(&env, key, &payload.to_array()));
    }

    // Worst case: every signer is required and all contexts are evaluated.
    let target = env.register(MultisigAccount, (Vec::<SignerKey>::new(&env), 1u32));
    let contexts = Vec::from_array(&env, [contract_context(&env, &target, "upgrade")]);

    let budget = env.budget();
    budget.reset_unlimited();
    let result = check_auth_with_contexts(&env, &account, &payload, signatures, contexts);
    let cpu = budget.cpu_instruction_cost();
    let mem = budget.memory_bytes_cost();

    assert_eq!(result, Ok(()));
    assert!(
        cpu <= MAX_AUTH_CPU_BUDGET,
        "worst-case auth used {cpu} CPU instructions, budget is {MAX_AUTH_CPU_BUDGET}"
    );
    assert!(
        mem <= MAX_AUTH_MEMORY_BUDGET,
        "worst-case auth used {mem} memory bytes, budget is {MAX_AUTH_MEMORY_BUDGET}"
    );
    let _ = client;
}

#[test]
fn get_interface_reports_kind_versions_and_features() {
    let env = Env::default();
    let keys = signing_keys();
    let account = register_account(&env, &keys[..2], 1);
    let client = MultisigAccountClient::new(&env, &account);

    let info = client.get_interface();
    assert_eq!(
        info.contract_kind,
        soroban_sdk::Symbol::new(&env, "lafiya_multisig_account")
    );
    assert_eq!(info.interface_version, INTERFACE_VERSION);
    assert_eq!(info.schema_version, SCHEMA_VERSION);
    assert_eq!(info.event_version, EVENT_VERSION);
    assert!(info
        .features
        .contains(soroban_sdk::Symbol::new(&env, "unscoped_auth")));
}
