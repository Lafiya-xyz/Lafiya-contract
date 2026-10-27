//! Deterministic salt derivation for [`commit_v1_salted`](crate::commit_v1_salted)
//! (ADR-0018).
//!
//! A record's salt is derived from a patient-held master secret with
//! HKDF-SHA256 (RFC 5869), so it can be recomputed from the master secret
//! alone after device loss, yet cannot be guessed without it:
//!
//! ```text
//! prk  = HKDF-Extract(salt = SALT_DERIVATION_EXTRACT_SALT, ikm = master_secret)
//! info = SALT_INFO_TAG || SALT_DERIVATION_V1
//!        || u32be(len(record_id)) || record_id || u32be(salt_version)
//! salt = HKDF-Expand(prk, info, 32)
//! ```
//!
//! Rotating a record's salt means deriving it at the next `salt_version` and
//! superseding the old attestation. The Python (`reference-py/salt.py`) and
//! TypeScript (`reference-ts/salt.ts`) references are checked against the
//! same fixture, `vectors/salt-derivation-vectors.json`.

use crate::Salt;
use core::fmt;
use sha2::{Digest, Sha256};

/// HKDF-Extract salt: domain-separates this derivation from any other use of
/// the master secret.
pub const SALT_DERIVATION_EXTRACT_SALT: &[u8] = b"lafiya:salt-derivation:v1";
/// Leading tag of the HKDF `info` for record salts.
pub const SALT_INFO_TAG: &[u8] = b"lafiya:record-salt";
/// HKDF `info` for the master-secret fingerprint.
pub const FINGERPRINT_INFO: &[u8] = b"lafiya:master-secret-fingerprint";
/// Version byte of the `info` layout implemented here.
pub const SALT_DERIVATION_V1: u8 = 0x01;

pub const MIN_MASTER_SECRET_LEN: usize = 32;
pub const MAX_MASTER_SECRET_LEN: usize = 64;
pub const MAX_RECORD_ID_LEN: usize = 256;
pub const FINGERPRINT_LEN: usize = 8;

const BLOCK: usize = 64;
const HASH_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaltDerivationError {
    /// The master secret is outside `MIN_MASTER_SECRET_LEN..=MAX_MASTER_SECRET_LEN`.
    MasterSecretLength { len: usize },
    /// The record id is empty or longer than `MAX_RECORD_ID_LEN`.
    RecordIdLength { len: usize },
    /// Salt versions start at 1.
    ZeroSaltVersion,
}

impl fmt::Display for SaltDerivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaltDerivationError::MasterSecretLength { len } => write!(
                f,
                "master secret must be {MIN_MASTER_SECRET_LEN}..={MAX_MASTER_SECRET_LEN} bytes, got {len}"
            ),
            SaltDerivationError::RecordIdLength { len } => {
                write!(f, "record id must be 1..={MAX_RECORD_ID_LEN} bytes, got {len}")
            }
            SaltDerivationError::ZeroSaltVersion => write!(f, "salt version must be at least 1"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SaltDerivationError {}

/// HMAC-SHA256 over the concatenation of `parts`.
fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; HASH_LEN] {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..HASH_LEN].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    for part in parts {
        inner.update(part);
    }
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// RFC 5869 HKDF-Extract with HMAC-SHA256.
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; HASH_LEN] {
    hmac_sha256(salt, &[ikm])
}

/// RFC 5869 HKDF-Expand with HMAC-SHA256, filling `okm`.
///
/// # Panics
/// If `okm` is longer than 255 * 32 bytes, the HKDF output limit.
pub fn hkdf_expand(prk: &[u8; HASH_LEN], info: &[u8], okm: &mut [u8]) {
    assert!(
        okm.len() <= 255 * HASH_LEN,
        "HKDF-SHA256 output is limited to 8160 bytes"
    );
    let mut previous: [u8; HASH_LEN] = [0; HASH_LEN];
    for (i, chunk) in okm.chunks_mut(HASH_LEN).enumerate() {
        let counter = [i as u8 + 1];
        let t: &[u8] = if i == 0 { &[] } else { &previous };
        previous = hmac_sha256(prk, &[t, info, &counter]);
        chunk.copy_from_slice(&previous[..chunk.len()]);
    }
}

fn prk(master_secret: &[u8]) -> Result<[u8; HASH_LEN], SaltDerivationError> {
    if !(MIN_MASTER_SECRET_LEN..=MAX_MASTER_SECRET_LEN).contains(&master_secret.len()) {
        return Err(SaltDerivationError::MasterSecretLength {
            len: master_secret.len(),
        });
    }
    Ok(hkdf_extract(SALT_DERIVATION_EXTRACT_SALT, master_secret))
}

/// Writes the HKDF `info` for `record_id` at `salt_version` into `out`,
/// returning its length.
fn write_info(
    record_id: &[u8],
    salt_version: u32,
    out: &mut [u8; SALT_INFO_TAG.len() + 1 + 4 + MAX_RECORD_ID_LEN + 4],
) -> Result<usize, SaltDerivationError> {
    if record_id.is_empty() || record_id.len() > MAX_RECORD_ID_LEN {
        return Err(SaltDerivationError::RecordIdLength {
            len: record_id.len(),
        });
    }
    if salt_version == 0 {
        return Err(SaltDerivationError::ZeroSaltVersion);
    }
    let mut n = 0;
    for part in [
        SALT_INFO_TAG,
        &[SALT_DERIVATION_V1],
        &(record_id.len() as u32).to_be_bytes(),
        record_id,
        &salt_version.to_be_bytes(),
    ] {
        out[n..n + part.len()].copy_from_slice(part);
        n += part.len();
    }
    Ok(n)
}

/// Derive the salt for `record_id` (an opaque id, UTF-8 NFC when textual) at
/// `salt_version` (starting at 1) from a patient master secret of 32 to 64
/// uniformly random bytes.
pub fn derive_salt(
    master_secret: &[u8],
    record_id: &[u8],
    salt_version: u32,
) -> Result<Salt, SaltDerivationError> {
    let prk = prk(master_secret)?;
    let mut info = [0u8; SALT_INFO_TAG.len() + 1 + 4 + MAX_RECORD_ID_LEN + 4];
    let len = write_info(record_id, salt_version, &mut info)?;
    let mut salt = [0u8; 32];
    hkdf_expand(&prk, &info[..len], &mut salt);
    Ok(Salt::from_bytes(salt))
}

/// A public, non-secret 8-byte identifier of a master secret. Stored next to
/// commitments so that, after a master-secret rotation, a wallet knows which
/// secret re-derives a given record's salt.
pub fn master_secret_fingerprint(
    master_secret: &[u8],
) -> Result<[u8; FINGERPRINT_LEN], SaltDerivationError> {
    let prk = prk(master_secret)?;
    let mut out = [0u8; FINGERPRINT_LEN];
    hkdf_expand(&prk, FINGERPRINT_INFO, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commit_v1_salted, FieldValue};
    use serde::Deserialize;
    use std::path::Path;

    #[derive(Deserialize)]
    struct Fixture {
        hkdf_rfc5869: Vec<HkdfCase>,
        vectors: Vec<Vector>,
        invalid: Vec<Invalid>,
    }

    #[derive(Deserialize)]
    struct HkdfCase {
        name: String,
        ikm_hex: String,
        salt_hex: String,
        info_hex: String,
        length: usize,
        okm_hex: String,
    }

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        master_secret_hex: String,
        record_id: String,
        salt_version: u32,
        info_hex: String,
        salt_hex: String,
        fingerprint_hex: String,
        #[serde(default)]
        fields: Vec<JsonField>,
        commitment_hex: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "lowercase")]
    enum JsonField {
        Text { value: String },
        Int64 { value: String },
    }

    #[derive(Deserialize)]
    struct Invalid {
        name: String,
        master_secret_hex: String,
        record_id: String,
        salt_version: u32,
        error: String,
    }

    fn fixture() -> Fixture {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/salt-derivation-vectors.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn hkdf_matches_rfc5869() {
        for tc in fixture().hkdf_rfc5869 {
            let prk = hkdf_extract(
                &hex::decode(&tc.salt_hex).unwrap(),
                &hex::decode(&tc.ikm_hex).unwrap(),
            );
            let mut okm = vec![0u8; tc.length];
            hkdf_expand(&prk, &hex::decode(&tc.info_hex).unwrap(), &mut okm);
            assert_eq!(hex::encode(okm), tc.okm_hex, "{}", tc.name);
        }
    }

    #[test]
    fn hmac_hashes_long_keys() {
        // RFC 4231 test case 6: a 131-byte key is hashed before use.
        let mac = hmac_sha256(
            &[0xaa; 131],
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"],
        );
        assert_eq!(
            hex::encode(mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn matches_shared_vectors() {
        for v in fixture().vectors {
            let secret = hex::decode(&v.master_secret_hex).unwrap();
            let record_id = v.record_id.as_bytes();

            let mut info = [0u8; SALT_INFO_TAG.len() + 1 + 4 + MAX_RECORD_ID_LEN + 4];
            let len = write_info(record_id, v.salt_version, &mut info).unwrap();
            assert_eq!(hex::encode(&info[..len]), v.info_hex, "info: {}", v.name);

            let salt = derive_salt(&secret, record_id, v.salt_version).unwrap();
            assert_eq!(hex::encode(salt.as_bytes()), v.salt_hex, "salt: {}", v.name);
            assert_eq!(
                hex::encode(master_secret_fingerprint(&secret).unwrap()),
                v.fingerprint_hex,
                "fingerprint: {}",
                v.name
            );

            if let Some(expected) = v.commitment_hex {
                let fields: Vec<FieldValue> = v
                    .fields
                    .into_iter()
                    .map(|f| match f {
                        JsonField::Text { value } => FieldValue::Text(value),
                        JsonField::Int64 { value } => FieldValue::Int64(value.parse().unwrap()),
                    })
                    .collect();
                let commitment = commit_v1_salted(&salt, &fields).unwrap();
                assert_eq!(hex::encode(commitment), expected, "commitment: {}", v.name);
            }
        }
    }

    #[test]
    fn rejects_invalid_inputs() {
        for v in fixture().invalid {
            let err = derive_salt(
                &hex::decode(&v.master_secret_hex).unwrap(),
                v.record_id.as_bytes(),
                v.salt_version,
            )
            .unwrap_err();
            let kind = match err {
                SaltDerivationError::MasterSecretLength { .. } => "master_secret_length",
                SaltDerivationError::RecordIdLength { .. } => "record_id_length",
                SaltDerivationError::ZeroSaltVersion => "salt_version",
            };
            assert_eq!(kind, v.error, "{}", v.name);
        }
    }
}
