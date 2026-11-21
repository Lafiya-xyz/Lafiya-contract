//! Dispute signature v2 (docs/specs/dispute-signature-v2.md): shared vectors
//! and replay regression tests for threat T-152.

use crate::{dispute_message_v2, AttestationRegistry, AttestationRegistryClient, Error};
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address, BytesN, Env};

const VECTORS: &str = include_str!("../../../docs/specs/vectors/dispute-v2-vectors.json");

fn hex32(env: &Env, s: &str) -> BytesN<32> {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    BytesN::from_array(env, &out)
}

struct Fixture {
    env: Env,
    client: AttestationRegistryClient<'static>,
    key: SigningKey,
    signer: Address,
}

fn fixture() -> Fixture {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let id = env.register(AttestationRegistry, ());
    let client = AttestationRegistryClient::new(&env, &id);
    let signer = Address::generate(&env);
    Fixture {
        env,
        client,
        key: SigningKey::from_bytes(&[7u8; 32]),
        signer,
    }
}

impl Fixture {
    fn signer_key(&self) -> BytesN<32> {
        BytesN::from_array(&self.env, &self.key.verifying_key().to_bytes())
    }

    /// Sign the v2 message as seen by `contract` on the current network.
    fn sign(&self, contract: &Address, record: &BytesN<32>, nonce: u64, exp: u64) -> BytesN<64> {
        let msg = dispute_message_v2(
            &self.env,
            &self.env.ledger().network_id(),
            contract,
            record,
            &self.signer,
            &self.signer_key(),
            nonce,
            exp,
        );
        BytesN::from_array(&self.env, &self.key.sign(&msg.to_array()).to_bytes())
    }
}

#[test]
fn dispute_v2_matches_shared_vectors() {
    let env = Env::default();
    let doc: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(doc["domain"], crate::DISPUTE_DOMAIN);
    for v in doc["vectors"].as_array().unwrap() {
        let s = |k: &str| v[k].as_str().unwrap();
        let got = dispute_message_v2(
            &env,
            &hex32(&env, s("network_id_hex")),
            &Address::from_str(&env, s("contract")),
            &hex32(&env, s("record_hash_hex")),
            &Address::from_str(&env, s("signer")),
            &hex32(&env, s("signer_key_hex")),
            s("nonce").parse().unwrap(),
            s("expires_at").parse().unwrap(),
        );
        assert_eq!(got, hex32(&env, s("message_hex")), "vector {}", s("name"));
    }
}

#[test]
fn sec_t152_dispute_valid_signature_accepted() {
    let f = fixture();
    let record = BytesN::from_array(&f.env, &[1u8; 32]);
    let sig = f.sign(&f.client.address, &record, 1, 2_000);
    f.client
        .dispute_attestation(&f.signer, &f.signer_key(), &record, &1, &2_000, &sig);
    assert!(f.client.is_disputed(&record, &f.signer));
}

#[test]
fn sec_t152_dispute_rejects_other_contract_instance() {
    let f = fixture();
    let other = f.env.register(AttestationRegistry, ());
    let record = BytesN::from_array(&f.env, &[2u8; 32]);
    let sig = f.sign(&other, &record, 1, 2_000);
    assert!(f
        .client
        .try_dispute_attestation(&f.signer, &f.signer_key(), &record, &1, &2_000, &sig)
        .is_err());
}

#[test]
fn sec_t152_dispute_rejects_other_network() {
    let f = fixture();
    let record = BytesN::from_array(&f.env, &[3u8; 32]);
    let sig = f.sign(&f.client.address, &record, 1, 2_000);
    f.env.ledger().with_mut(|l| l.network_id = [0x42; 32]);
    assert!(f
        .client
        .try_dispute_attestation(&f.signer, &f.signer_key(), &record, &1, &2_000, &sig)
        .is_err());
}

#[test]
fn sec_t152_dispute_rejects_different_signer_key() {
    let f = fixture();
    let record = BytesN::from_array(&f.env, &[4u8; 32]);
    let sig = f.sign(&f.client.address, &record, 1, 2_000);
    let wrong = BytesN::from_array(
        &f.env,
        &SigningKey::from_bytes(&[8u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    assert!(f
        .client
        .try_dispute_attestation(&f.signer, &wrong, &record, &1, &2_000, &sig)
        .is_err());
}

#[test]
fn sec_t152_dispute_rejects_expired_message() {
    let f = fixture();
    let record = BytesN::from_array(&f.env, &[5u8; 32]);
    let sig = f.sign(&f.client.address, &record, 1, 999);
    assert_eq!(
        f.client
            .try_dispute_attestation(&f.signer, &f.signer_key(), &record, &1, &999, &sig),
        Err(Ok(Error::DisputeExpired))
    );
}

#[test]
fn sec_t152_dispute_rejects_reused_nonce() {
    let f = fixture();
    let first = BytesN::from_array(&f.env, &[6u8; 32]);
    let second = BytesN::from_array(&f.env, &[7u8; 32]);
    let sig = f.sign(&f.client.address, &first, 9, 2_000);
    f.client
        .dispute_attestation(&f.signer, &f.signer_key(), &first, &9, &2_000, &sig);
    let sig2 = f.sign(&f.client.address, &second, 9, 2_000);
    assert_eq!(
        f.client
            .try_dispute_attestation(&f.signer, &f.signer_key(), &second, &9, &2_000, &sig2),
        Err(Ok(Error::DisputeNonceUsed))
    );
}
