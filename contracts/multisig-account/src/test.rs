extern crate std;

use super::*;
use ed25519_dalek::{Signer as _, SigningKey};
use soroban_sdk::{auth::Context, BytesN, Env, IntoVal, Vec};

pub(crate) fn signing_keys() -> std::vec::Vec<SigningKey> {
    let mut keys = std::vec![
        SigningKey::from_bytes(&[1; 32]),
        SigningKey::from_bytes(&[2; 32]),
        SigningKey::from_bytes(&[3; 32]),
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
            public_key: BytesN::from_array(env, &key.verifying_key().to_bytes()),
            signature: BytesN::from_array(env, &key.sign(payload).to_bytes()),
        });
    }
    signatures
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
