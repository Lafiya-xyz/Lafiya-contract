//! Reference implementation of the Lafiya Record Commitment v1 (LRC-1)
//! canonicalization and domain-separated hashing scheme.
//!
//! This crate is the Rust half of the cross-language prototype produced for
//! the record-commitment spike (see
//! `docs/adr/0008-record-commitment-canonicalization.md`). The TypeScript
//! half lives in `reference-ts/lrc1.ts`; both implementations are checked
//! against the same fixture in `vectors/lrc1-test-vectors.json`.
//!
//! # Scheme summary
//!
//! A record is a fixed, schema-defined ordered list of [`FieldValue`]s.
//! Each field encodes to a tag byte followed by its value (see
//! [`encode_payload`] for the exact byte layout). The commitment is:
//!
//! ```text
//! SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)
//! ```
//!
//! Field order is part of the schema and is never sorted at runtime — this
//! avoids any dependency on Unicode key-collation rules. There are no
//! floating-point values, so there is no cross-language number-formatting
//! ambiguity to resolve. `Absent`, `Null`, and every value type get a
//! distinct tag, so "field omitted" and "field explicitly null" never
//! collide.
//!
//! # Unicode normalization
//!
//! [`FieldValue::Text`] is encoded as-is: this crate does not perform
//! Unicode normalization itself, to avoid pulling in a Unicode-tables
//! dependency for a prototype. Callers MUST pass Unicode Normalization Form
//! C (NFC) strings — two different byte sequences that render identically
//! (e.g. a precomposed vs. combining-mark accent) will otherwise produce
//! different commitments. The TypeScript reference normalizes via the
//! built-in `String.prototype.normalize("NFC")`. A production Rust
//! implementation should validate or normalize input with a Unicode-aware
//! crate before encoding; see the ADR's follow-up section.

#![cfg_attr(not(any(test, feature = "std")), no_std)]

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use core::fmt;

#[cfg(any(test, feature = "std"))]
use std;

use sha2::{Digest, Sha256};

/// Schema-level size limits to prevent memory exhaustion
pub const MAX_FIELD_BYTES: usize = 65_536;   // 64 KiB per field
pub const MAX_FIELDS: usize = 256;            // 256 fields max
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576; // 1 MiB total

/// Encoding error type for fallible operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// A field exceeds MAX_FIELD_BYTES
    FieldTooLarge { size: usize },
    /// Payload exceeds MAX_PAYLOAD_BYTES
    PayloadTooLarge { size: usize },
    /// Too many fields (exceeds MAX_FIELDS)
    TooManyFields { count: usize },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EncodeError::FieldTooLarge { size } => {
                write!(f, "field size {} exceeds limit {}", size, MAX_FIELD_BYTES)
            }
            EncodeError::PayloadTooLarge { size } => {
                write!(f, "payload size {} exceeds limit {}", size, MAX_PAYLOAD_BYTES)
            }
            EncodeError::TooManyFields { count } => {
                write!(f, "field count {} exceeds limit {}", count, MAX_FIELDS)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}

/// High-entropy salt for dictionary-attack resistance (32 bytes).
/// Required for [`commit_v1_salted`] to prevent brute-forcing low-entropy records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Salt([u8; 32]);

impl Salt {
    /// Create a salt from a fixed byte array (for re-verification).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Salt(bytes)
    }

    /// Get the salt as a byte slice for storage/transmission.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Get the salt as mutable bytes.
    pub fn as_bytes_mut(&mut self) -> &mut [u8; 32] {
        &mut self.0
    }
}

/// Domain-separation tag mixed into every LRC-1 commitment. Distinguishes a
/// Lafiya record commitment from a hash produced for an unrelated purpose
/// (e.g. a different protocol reusing SHA-256 over similarly shaped bytes).
pub const DOMAIN_TAG: &[u8] = b"lafiya:record-commitment";

/// Canonicalization scheme version implemented by this crate.
pub const VERSION_V1: u8 = 0x01;

/// Reserved version byte for commitments recorded before LRC-1 existed.
/// Never assigned to a canonicalization scheme — see the ADR's "Existing
/// commitment compatibility" section. A verifier must not attempt to
/// recompute a legacy commitment with this scheme.
pub const VERSION_LEGACY_UNVERSIONED: u8 = 0x00;

const TAG_ABSENT: u8 = 0x00;
const TAG_NULL: u8 = 0x01;
const TAG_TEXT: u8 = 0x10;
const TAG_INT64: u8 = 0x11;
const TAG_BOOL: u8 = 0x12;
const TAG_BYTES: u8 = 0x13;

/// A single record field in its fixed schema position.
///
/// `Absent` and `Null` are distinct: `Absent` means the source record does
/// not contain this field at all, `Null` means the field is present with an
/// explicit null value. Collapsing the two would let an attacker or a buggy
/// client silently reinterpret one record as another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// The field is not present in the source record.
    Absent,
    /// The field is present with an explicit null value.
    Null,
    /// A UTF-8 string. Must already be in Unicode Normalization Form C.
    Text(String),
    /// A signed 64-bit integer. Used for timestamps (Unix epoch seconds,
    /// UTC, no sub-second precision).
    Int64(i64),
    /// A boolean flag.
    Bool(bool),
    /// Raw bytes, e.g. a pre-hashed or salted opaque reference.
    Bytes(Vec<u8>),
}

impl FieldValue {
    fn encode(&self, out: &mut Vec<u8>) -> Result<(), EncodeError> {
        match self {
            FieldValue::Absent => {
                out.push(TAG_ABSENT);
                Ok(())
            }
            FieldValue::Null => {
                out.push(TAG_NULL);
                Ok(())
            }
            FieldValue::Text(s) => {
                out.push(TAG_TEXT);
                let bytes = s.as_bytes();
                if bytes.len() > MAX_FIELD_BYTES {
                    return Err(EncodeError::FieldTooLarge { size: bytes.len() });
                }
                out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                out.extend_from_slice(bytes);
                Ok(())
            }
            FieldValue::Int64(v) => {
                out.push(TAG_INT64);
                out.extend_from_slice(&v.to_be_bytes());
                Ok(())
            }
            FieldValue::Bool(b) => {
                out.push(TAG_BOOL);
                out.push(u8::from(*b));
                Ok(())
            }
            FieldValue::Bytes(b) => {
                out.push(TAG_BYTES);
                if b.len() > MAX_FIELD_BYTES {
                    return Err(EncodeError::FieldTooLarge { size: b.len() });
                }
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
                Ok(())
            }
        }
    }
}

/// Encode an ordered list of record fields into the LRC-1 canonical
/// payload. Field order is part of the schema and MUST be fixed by the
/// caller; this function does not sort or reorder fields.
/// Returns error if any field exceeds MAX_FIELD_BYTES or total payload exceeds MAX_PAYLOAD_BYTES.
pub fn encode_payload(fields: &[FieldValue]) -> Result<Vec<u8>, EncodeError> {
    if fields.len() > MAX_FIELDS {
        return Err(EncodeError::TooManyFields { count: fields.len() });
    }

    let mut out = Vec::new();
    for field in fields {
        field.encode(&mut out)?;

        if out.len() > MAX_PAYLOAD_BYTES {
            return Err(EncodeError::PayloadTooLarge { size: out.len() });
        }
    }
    Ok(out)
}

/// Compute the LRC-1 record commitment:
/// `SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)`.
/// Deprecated: use [`commit_v1_salted`] for dictionary-attack resistance.
#[deprecated(since = "0.2.0", note = "use commit_v1_salted for security")]
pub fn commit_v1(fields: &[FieldValue]) -> Result<[u8; 32], EncodeError> {
    let payload = encode_payload(fields)?;
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_TAG);
    hasher.update([VERSION_V1]);
    hasher.update(&payload);
    Ok(hasher.finalize().into())
}

/// Compute the LRC-1 record commitment with high-entropy salt:
/// `SHA-256(DOMAIN_TAG || VERSION_V1 || salt_field || canonical_payload)`.
/// The salt is encoded as a TAG_BYTES field prepended to the fields.
pub fn commit_v1_salted(salt: &Salt, fields: &[FieldValue]) -> Result<[u8; 32], EncodeError> {
    // Prepend salt as a mandatory first field
    let mut salted_fields = Vec::with_capacity(fields.len() + 1);
    salted_fields.push(FieldValue::Bytes(salt.0.to_vec()));
    salted_fields.extend_from_slice(fields);

    let payload = encode_payload(&salted_fields)?;
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_TAG);
    hasher.update([VERSION_V1]);
    hasher.update(&payload);
    Ok(hasher.finalize().into())
}

/// Decode a LRC-1 canonical payload back into fields (round-trip verification).
/// Returns error if payload is malformed or exceeds size limits.
pub fn decode_payload(mut payload: &[u8]) -> Result<Vec<FieldValue>, EncodeError> {
    let mut fields = Vec::new();

    while !payload.is_empty() {
        if fields.len() >= MAX_FIELDS {
            return Err(EncodeError::TooManyFields { count: fields.len() + 1 });
        }

        let tag = payload[0];
        payload = &payload[1..];

        let field = match tag {
            TAG_ABSENT => FieldValue::Absent,
            TAG_NULL => FieldValue::Null,
            TAG_TEXT => {
                if payload.len() < 4 {
                    return Err(EncodeError::PayloadTooLarge { size: 0 });
                }
                let len = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                payload = &payload[4..];

                if len > MAX_FIELD_BYTES || payload.len() < len {
                    return Err(EncodeError::FieldTooLarge { size: len });
                }

                let text = String::from_utf8_lossy(&payload[..len]).to_string();
                payload = &payload[len..];
                FieldValue::Text(text)
            }
            TAG_INT64 => {
                if payload.len() < 8 {
                    return Err(EncodeError::PayloadTooLarge { size: 0 });
                }
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&payload[..8]);
                payload = &payload[8..];
                FieldValue::Int64(i64::from_be_bytes(bytes))
            }
            TAG_BOOL => {
                let val = payload[0] != 0;
                payload = &payload[1..];
                FieldValue::Bool(val)
            }
            TAG_BYTES => {
                if payload.len() < 4 {
                    return Err(EncodeError::PayloadTooLarge { size: 0 });
                }
                let len = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                payload = &payload[4..];

                if len > MAX_FIELD_BYTES || payload.len() < len {
                    return Err(EncodeError::FieldTooLarge { size: len });
                }

                let bytes = payload[..len].to_vec();
                payload = &payload[len..];
                FieldValue::Bytes(bytes)
            }
            _ => return Err(EncodeError::PayloadTooLarge { size: 0 }),
        };

        fields.push(field);
    }

    Ok(fields)
}

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Soroban-native commitment using host crypto (requires soroban feature)
#[cfg(feature = "soroban")]
pub mod soroban {
    use super::*;
    use soroban_sdk::{Bytes, BytesN, Env};

    /// Compute LRC-1 commitment using Soroban's host-native SHA-256.
    /// Equivalent to commit_v1_salted but uses env.crypto().sha256 for metered hashing.
    pub fn commit_v1_soroban_salted(
        env: &Env,
        salt: &Salt,
        fields: &[FieldValue],
    ) -> Result<BytesN<32>, EncodeError> {
        // Prepend salt as first field
        let mut salted_fields = Vec::with_capacity(fields.len() + 1);
        salted_fields.push(FieldValue::Bytes(salt.0.to_vec()));
        salted_fields.extend_from_slice(fields);

        let payload = encode_payload(&salted_fields)?;

        let mut preimage = Bytes::new(env);
        for byte in DOMAIN_TAG {
            preimage.push_back(*byte);
        }
        preimage.push_back(VERSION_V1);
        for byte in &payload {
            preimage.push_back(*byte);
        }

        Ok(env.crypto().sha256(&preimage))
    }

    /// Compute unsalted commitment using Soroban host crypto (for legacy/verification).
    #[deprecated(since = "0.2.0", note = "use commit_v1_soroban_salted for security")]
    pub fn commit_v1_soroban(
        env: &Env,
        fields: &[FieldValue],
    ) -> Result<BytesN<32>, EncodeError> {
        let payload = encode_payload(fields)?;

        let mut preimage = Bytes::new(env);
        for byte in DOMAIN_TAG {
            preimage.push_back(*byte);
        }
        preimage.push_back(VERSION_V1);
        for byte in &payload {
            preimage.push_back(*byte);
        }

        Ok(env.crypto().sha256(&preimage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Deserialize)]
    #[serde(tag = "kind", rename_all = "lowercase")]
    enum JsonField {
        Absent,
        Null,
        Text { value: String },
        Int64 { value: String },
        Bool { value: bool },
        Bytes { value: String },
    }

    impl From<JsonField> for FieldValue {
        fn from(f: JsonField) -> Self {
            match f {
                JsonField::Absent => FieldValue::Absent,
                JsonField::Null => FieldValue::Null,
                JsonField::Text { value } => FieldValue::Text(value),
                JsonField::Int64 { value } => {
                    FieldValue::Int64(value.parse().expect("valid i64 in fixture"))
                }
                JsonField::Bool { value } => FieldValue::Bool(value),
                JsonField::Bytes { value } => {
                    FieldValue::Bytes(hex::decode(value).expect("valid hex in fixture"))
                }
            }
        }
    }

    #[derive(Debug, Deserialize)]
    struct Vector {
        name: String,
        fields: Vec<JsonField>,
        payload_hex: String,
        commitment_hex: String,
    }

    fn load_vectors() -> Vec<Vector> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/lrc1-test-vectors.json");
        let raw = fs::read_to_string(path).expect("read test vector fixture");
        serde_json::from_str(&raw).expect("parse test vector fixture")
    }

    /// Every vector in the shared fixture (produced by the TypeScript
    /// reference implementation, cross-checked against an independent
    /// Python prototype) must reproduce byte-for-byte in Rust. This is the
    /// cross-language determinism proof the spike's acceptance criteria
    /// call for.
    #[test]
    fn matches_cross_language_test_vectors() {
        let vectors = load_vectors();
        assert!(!vectors.is_empty(), "fixture must not be empty");

        for vector in vectors {
            let fields: Vec<FieldValue> = vector.fields.into_iter().map(FieldValue::from).collect();

            let payload = encode_payload(&fields).expect("valid payload");
            assert_eq!(
                hex::encode(&payload),
                vector.payload_hex,
                "payload mismatch for vector `{}`",
                vector.name
            );

            let commitment = commit_v1(&fields).expect("valid commitment");
            assert_eq!(
                hex::encode(commitment),
                vector.commitment_hex,
                "commitment mismatch for vector `{}`",
                vector.name
            );
        }
    }

    #[test]
    fn absent_null_and_value_produce_different_commitments() {
        let base = |note: FieldValue| {
            vec![
                FieldValue::Text("lafiya.emergency_record".into()),
                FieldValue::Int64(1_765_900_800),
                note,
            ]
        };

        let absent = commit_v1(&base(FieldValue::Absent)).expect("valid");
        let null = commit_v1(&base(FieldValue::Null)).expect("valid");
        let value = commit_v1(&base(FieldValue::Text("note".into()))).expect("valid");

        assert_ne!(absent, null);
        assert_ne!(absent, value);
        assert_ne!(null, value);
    }

    #[test]
    fn field_order_is_significant() {
        let a = vec![
            FieldValue::Text("first".into()),
            FieldValue::Text("second".into()),
        ];
        let b = vec![
            FieldValue::Text("second".into()),
            FieldValue::Text("first".into()),
        ];

        assert_ne!(commit_v1(&a).expect("a"), commit_v1(&b).expect("b"));
    }

    #[test]
    fn commitment_is_deterministic() {
        let fields = vec![FieldValue::Bool(true), FieldValue::Int64(42)];
        assert_eq!(commit_v1(&fields).expect("v1"), commit_v1(&fields).expect("v2"));
    }

    #[test]
    fn salted_commitment_resists_dictionary_attacks() {
        let salt = Salt::from_bytes([42u8; 32]);
        let fields = vec![
            FieldValue::Text("genotype".into()),
            FieldValue::Text("SS".into()),
        ];

        let salted = commit_v1_salted(&salt, &fields).expect("valid");
        let unsalted = commit_v1(&fields).expect("valid");

        // Different commitments prove the salt affects the result
        assert_ne!(salted, unsalted);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = vec![
            FieldValue::Text("record".into()),
            FieldValue::Int64(1_234_567_890),
            FieldValue::Bool(true),
            FieldValue::Bytes(vec![1, 2, 3, 4]),
        ];

        let payload = encode_payload(&original).expect("valid encode");
        let decoded = decode_payload(&payload).expect("valid decode");

        assert_eq!(original, decoded);
    }

    #[test]
    fn field_size_limit_enforced() {
        let oversized = vec![FieldValue::Bytes(vec![0u8; MAX_FIELD_BYTES + 1])];
        let result = encode_payload(&oversized);
        assert!(matches!(result, Err(EncodeError::FieldTooLarge { .. })));
    }

    #[test]
    fn too_many_fields_rejected() {
        let many_fields = vec![FieldValue::Null; MAX_FIELDS + 1];
        let result = encode_payload(&many_fields);
        assert!(matches!(result, Err(EncodeError::TooManyFields { .. })));
    }
}
