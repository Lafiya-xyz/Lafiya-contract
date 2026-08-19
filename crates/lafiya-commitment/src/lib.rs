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

use sha2::{Digest, Sha256};

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
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            FieldValue::Absent => out.push(TAG_ABSENT),
            FieldValue::Null => out.push(TAG_NULL),
            FieldValue::Text(s) => {
                out.push(TAG_TEXT);
                let bytes = s.as_bytes();
                out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                out.extend_from_slice(bytes);
            }
            FieldValue::Int64(v) => {
                out.push(TAG_INT64);
                out.extend_from_slice(&v.to_be_bytes());
            }
            FieldValue::Bool(b) => {
                out.push(TAG_BOOL);
                out.push(u8::from(*b));
            }
            FieldValue::Bytes(b) => {
                out.push(TAG_BYTES);
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
            }
        }
    }
}

/// Encode an ordered list of record fields into the LRC-1 canonical
/// payload. Field order is part of the schema and MUST be fixed by the
/// caller; this function does not sort or reorder fields.
pub fn encode_payload(fields: &[FieldValue]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        field.encode(&mut out);
    }
    out
}

/// Compute the LRC-1 record commitment:
/// `SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)`.
pub fn commit_v1(fields: &[FieldValue]) -> [u8; 32] {
    let payload = encode_payload(fields);
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_TAG);
    hasher.update([VERSION_V1]);
    hasher.update(&payload);
    hasher.finalize().into()
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

            let payload = encode_payload(&fields);
            assert_eq!(
                hex::encode(&payload),
                vector.payload_hex,
                "payload mismatch for vector `{}`",
                vector.name
            );

            let commitment = commit_v1(&fields);
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

        let absent = commit_v1(&base(FieldValue::Absent));
        let null = commit_v1(&base(FieldValue::Null));
        let value = commit_v1(&base(FieldValue::Text("note".into())));

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

        assert_ne!(commit_v1(&a), commit_v1(&b));
    }

    #[test]
    fn commitment_is_deterministic() {
        let fields = vec![FieldValue::Bool(true), FieldValue::Int64(42)];
        assert_eq!(commit_v1(&fields), commit_v1(&fields));
    }
}
