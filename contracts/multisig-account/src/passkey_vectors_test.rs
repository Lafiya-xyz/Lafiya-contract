//! secp256r1 / passkey test vectors shared with `@lafiya/multisig-auth`
//! (`packages/multisig-auth/test/vectors/passkey-vectors.json`).
//!
//! The TypeScript suite checks that its DER-to-compact conversion and low-S
//! normalization reproduce `compact_hex` / `normalized_compact_hex`; this
//! module checks that the same bytes are what `__check_auth` accepts, and that
//! the malleable high-S form is rejected.
//!
//! Regenerate the file (deterministic, RFC 6979) with:
//! `cargo test -p multisig-account generate_passkey_vectors -- --ignored`

extern crate std;

use super::*;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature as P256Signature, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use soroban_sdk::{auth::Context, BytesN, Env, IntoVal, Vec};
use std::string::String;

const VECTORS: &str =
    include_str!("../../../packages/multisig-auth/test/vectors/passkey-vectors.json");
const VECTORS_PATH: &str = "../../packages/multisig-auth/test/vectors/passkey-vectors.json";

const RP_ID: &str = "localhost";
const ORIGIN: &str = "http://localhost:5173";
/// SubjectPublicKeyInfo DER prefix for an uncompressed P-256 point.
const P256_SPKI_PREFIX: &str = "3059301306072a8648ce3d020106082a8648ce3d030107034200";

fn hex_bytes<const N: usize>(value: &Value) -> [u8; N] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

fn vectors() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

fn register_p256_account(env: &Env, public_key: &[u8; 65]) -> soroban_sdk::Address {
    let keys = crate::test::signing_keys();
    let account = crate::test::register_account(env, &keys[..1], 1);
    let client = MultisigAccountClient::new(env, &account);
    let mut signers = Vec::new(env);
    signers.push_back(SignerConfig {
        public_key: SignerKey::Secp256r1(BytesN::from_array(env, public_key)),
        weight: 1,
        role: None,
    });
    env.mock_all_auths();
    client.set_policy(&signers, &1, &1, &Vec::new(env));
    env.set_auths(&[]);
    account
}

fn check_auth(
    env: &Env,
    account: &soroban_sdk::Address,
    payload: &[u8; 32],
    public_key: &[u8; 65],
    signature: &[u8; 64],
) -> Result<(), Result<Error, soroban_sdk::InvokeError>> {
    let mut signatures = Vec::new(env);
    signatures.push_back(Signature {
        public_key: SignerKey::Secp256r1(BytesN::from_array(env, public_key)),
        signature: BytesN::from_array(env, signature),
    });
    env.try_invoke_contract_check_auth::<Error>(
        account,
        &BytesN::from_array(env, payload),
        signatures.into_val(env),
        &Vec::<Context>::new(env),
    )
}

#[test]
fn prehash_vectors_verify_only_in_low_s_form() {
    let file = vectors();
    let public_key: [u8; 65] = hex_bytes(&file["public_key_uncompressed_hex"]);
    let mut checked = 0;
    for vector in file["vectors"].as_array().unwrap() {
        if vector["kind"] != "prehash" {
            continue;
        }
        let env = Env::default();
        let account = register_p256_account(&env, &public_key);
        let payload: [u8; 32] = hex_bytes(&vector["payload_hex"]);
        let normalized: [u8; 64] = hex_bytes(&vector["normalized_compact_hex"]);
        let compact: [u8; 64] = hex_bytes(&vector["compact_hex"]);

        assert_eq!(
            check_auth(&env, &account, &payload, &public_key, &normalized),
            Ok(()),
            "{}",
            vector["name"]
        );
        if vector["low_s"] == false {
            assert!(
                check_auth(&env, &account, &payload, &public_key, &compact).is_err(),
                "{}: high-S signature must be rejected",
                vector["name"]
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 2);
}

/// A WebAuthn assertion signs `SHA-256(authenticatorData ||
/// SHA-256(clientDataJSON))`, not the auth payload itself. `__check_auth`
/// accepts the vectors when that digest is the signature payload, but
/// rejects them against the auth payload the challenge was built from, so a
/// raw passkey assertion does not authorize yet. This pins that gap until
/// the contract verifies the WebAuthn envelope (see the package README).
#[test]
fn webauthn_vectors_verify_over_the_assertion_digest_only() {
    let file = vectors();
    let public_key: [u8; 65] = hex_bytes(&file["public_key_uncompressed_hex"]);
    let mut checked = 0;
    for vector in file["vectors"].as_array().unwrap() {
        if vector["kind"] != "webauthn" {
            continue;
        }
        let env = Env::default();
        let account = register_p256_account(&env, &public_key);
        let digest: [u8; 32] = hex_bytes(&vector["signed_digest_hex"]);
        let payload: [u8; 32] = hex_bytes(&vector["payload_hex"]);
        let normalized: [u8; 64] = hex_bytes(&vector["normalized_compact_hex"]);

        assert_eq!(
            check_auth(&env, &account, &digest, &public_key, &normalized),
            Ok(()),
            "{}",
            vector["name"]
        );
        assert!(check_auth(&env, &account, &payload, &public_key, &normalized).is_err());
        checked += 1;
    }
    assert_eq!(checked, 2);
}

fn signature_pair(key: &SigningKey, digest: &[u8; 32]) -> (P256Signature, P256Signature) {
    let signature: P256Signature = key.sign_prehash(digest).unwrap();
    let low = signature.normalize_s().unwrap_or(signature);
    let (r, s) = low.split_scalars();
    let high = P256Signature::from_scalars(r.to_bytes(), (-*s).to_bytes()).unwrap();
    (low, high)
}

fn vector_entries(
    name: &str,
    low: &P256Signature,
    high: &P256Signature,
    base: Value,
) -> [Value; 2] {
    let normalized = hex::encode(low.to_bytes());
    [(low, true, "low-s"), (high, false, "high-s")].map(|(sig, low_s, suffix)| {
        let mut entry = base.clone();
        let object = entry.as_object_mut().unwrap();
        object.insert("name".into(), json!(std::format!("{name}-{suffix}")));
        object.insert("low_s".into(), json!(low_s));
        object.insert(
            "der_hex".into(),
            json!(hex::encode(sig.to_der().as_bytes())),
        );
        object.insert("compact_hex".into(), json!(hex::encode(sig.to_bytes())));
        object.insert("normalized_compact_hex".into(), json!(normalized));
        entry
    })
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[test]
#[ignore = "regenerates the shared vector file"]
fn generate_passkey_vectors() {
    // Test-only key. The vector file carries only its public key.
    let key = SigningKey::from_bytes(&[7u8; 32].into()).unwrap();
    let public_key = key.verifying_key().to_encoded_point(false);
    let public_key_hex = hex::encode(public_key.as_bytes());

    let payload: [u8; 32] = Sha256::digest(b"lafiya multisig-auth passkey vector 1").into();
    let (low, high) = signature_pair(&key, &payload);
    let mut vectors = std::vec::Vec::new();
    vectors.extend(vector_entries(
        "prehash",
        &low,
        &high,
        json!({ "kind": "prehash", "payload_hex": hex::encode(payload) }),
    ));

    // authenticatorData: rpIdHash || flags (UP | UV) || signCount = 1
    let mut authenticator_data = Sha256::digest(RP_ID.as_bytes()).to_vec();
    authenticator_data.push(0x05);
    authenticator_data.extend_from_slice(&1u32.to_be_bytes());
    let client_data_json = std::format!(
        r#"{{"type":"webauthn.get","challenge":"{}","origin":"{ORIGIN}","crossOrigin":false}}"#,
        base64url(&payload)
    );
    let mut message = authenticator_data.clone();
    message.extend_from_slice(&Sha256::digest(client_data_json.as_bytes()));
    let digest: [u8; 32] = Sha256::digest(&message).into();
    let (low, high) = signature_pair(&key, &digest);
    vectors.extend(vector_entries(
        "webauthn",
        &low,
        &high,
        json!({
            "kind": "webauthn",
            "payload_hex": hex::encode(payload),
            "rp_id": RP_ID,
            "origin": ORIGIN,
            "authenticator_data_hex": hex::encode(&authenticator_data),
            "client_data_json": client_data_json,
            "signed_digest_hex": hex::encode(digest),
        }),
    ));

    let file = json!({
        "description": "secp256r1 vectors shared by @lafiya/multisig-auth and multisig-account; generated by contracts/multisig-account/src/passkey_vectors_test.rs",
        "curve": "P-256",
        "public_key_uncompressed_hex": public_key_hex,
        "public_key_spki_hex": std::format!("{P256_SPKI_PREFIX}{public_key_hex}"),
        "vectors": vectors,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_PATH);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(&file).unwrap() + "\n").unwrap();
}
