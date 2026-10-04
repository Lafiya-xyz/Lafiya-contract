//! LRC-2 — Per-field salted-hash commitment for selective disclosure.
//!
//! # Scheme summary
//!
//! LRC-2 extends LRC-1 with per-field salts and an independent leaf hash per
//! field, so a holder can disclose an arbitrary subset of fields and the
//! verifier can check those fields against the attested root without learning
//! anything about the undisclosed ones (other than their leaf hashes, which
//! are computationally independent of the field value when the salt is secret).
//!
//! ```text
//! leaf_i  = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt_i[32] || encode_lrc1(field_i))
//! root    = SHA-256("lafiya:lrc2:root" || leaf_0 || leaf_1 || … || leaf_{n-1})
//! ```
//!
//! - `encode_lrc1(field_i)` is the LRC-1 tag+length encoding of a single field
//!   (re-used from [`super::encode_payload`]).
//! - `salt_i` is a 32-byte random value per field, generated at record creation
//!   time and kept secret by the patient's app. Without the salt, a leaf is
//!   indistinguishable from a random 32-byte value; with it, any verifier can
//!   recompute the leaf and confirm it belongs to the root.
//! - `u8(i)` is the zero-based field index, preventing two fields with the same
//!   value from producing the same leaf hash regardless of salt.
//!
//! The `root` is the 32-byte value stored on-chain via `attest(attester, root)`.
//!
//! # Usage
//!
//! ```no_run
//! # #[cfg(feature = "lrc2")] {
//! use lafiya_commitment::lrc2::{commit_lrc2, make_leaf, make_root, verify_disclosure, Disclosure};
//! use lafiya_commitment::FieldValue;
//!
//! let fields = vec![
//!     FieldValue::Text("O+".into()),
//!     FieldValue::Text("AS".into()),
//!     FieldValue::Text("penicillin".into()),
//! ];
//! let salts: Vec<[u8; 32]> = (0..fields.len())
//!     .map(|i| [i as u8; 32])
//!     .collect();
//!
//! let (root, leaves) = commit_lrc2(&fields, &salts);
//!
//! // Disclose only field 0 (blood group).
//! let disclosures = vec![
//!     Disclosure { index: 0, salt: salts[0], field: fields[0].clone() },
//! ];
//! let undisclosed: Vec<[u8; 32]> = leaves[1..].to_vec();
//! assert!(verify_disclosure(&root, &disclosures, &undisclosed));
//! # }
//! ```
//!
//! # Version byte
//!
//! LRC-2 uses version byte `0x02` in contexts where a version must be surfaced
//! (e.g. the QR payload, see ADR-0013). The version is intentionally **not**
//! mixed into the root hash itself — the on-chain slot is opaque and the
//! version is carried by the application layer.
//!
//! See [`docs/adr/0012-lrc2-selective-disclosure.md`][adr] and [Issue #388].
//!
//! [adr]: ../../docs/adr/0012-lrc2-selective-disclosure.md
//! [Issue #388]: https://github.com/Lafiya-xyz/Lafiya-contract/issues/388

use sha2::{Digest, Sha256};

use crate::{encode_payload, FieldValue};

/// Domain-separation tag for LRC-2 leaf hashes.
pub const DOMAIN_LEAF: &[u8] = b"lafiya:lrc2:leaf";

/// Domain-separation tag for the LRC-2 root hash.
pub const DOMAIN_ROOT: &[u8] = b"lafiya:lrc2:root";

/// LRC-2 version byte (carried in application-layer contexts, not hashed into root).
pub const VERSION_LRC2: u8 = 0x02;

/// A single disclosed field together with its position and salt.
///
/// A verifier holds a list of `Disclosure`s (the revealed subset) plus the
/// raw leaf hashes of the undisclosed fields. From these it can reconstruct
/// every leaf and recompute the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disclosure {
    /// Zero-based field index within the schema.
    pub index: u8,
    /// Per-field random salt (32 bytes, caller-generated).
    pub salt: [u8; 32],
    /// The disclosed field value.
    pub field: FieldValue,
}

/// Compute the leaf hash for a single field.
///
/// ```text
/// leaf_i = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt[32] || encode_lrc1(field))
/// ```
pub fn make_leaf(index: u8, salt: &[u8; 32], field: &FieldValue) -> [u8; 32] {
    let encoded = encode_payload(std::slice::from_ref(field));
    let mut h = Sha256::new();
    h.update(DOMAIN_LEAF);
    h.update([index]);
    h.update(salt);
    h.update(&encoded);
    h.finalize().into()
}

/// Compute the root hash from an ordered slice of per-field leaves.
///
/// ```text
/// root = SHA-256("lafiya:lrc2:root" || leaf_0 || leaf_1 || … || leaf_{n-1})
/// ```
///
/// The caller must supply leaves in schema order (index 0, 1, 2, …).
pub fn make_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DOMAIN_ROOT);
    for leaf in leaves {
        h.update(leaf);
    }
    h.finalize().into()
}

/// Compute the LRC-2 root commitment and return it together with the
/// individual leaf hashes.
///
/// `fields` and `salts` must have the same length; each `salts[i]` is the
/// per-field random salt for `fields[i]`.
///
/// # Panics
///
/// Panics if `fields.len() != salts.len()` or if either is empty.
pub fn commit_lrc2(fields: &[FieldValue], salts: &[[u8; 32]]) -> ([u8; 32], Vec<[u8; 32]>) {
    assert_eq!(
        fields.len(),
        salts.len(),
        "fields and salts must have the same length"
    );
    assert!(!fields.is_empty(), "field list must not be empty");
    assert!(
        fields.len() <= 256,
        "LRC-2 supports at most 256 fields (index fits in u8)"
    );

    let leaves: Vec<[u8; 32]> = fields
        .iter()
        .zip(salts.iter())
        .enumerate()
        .map(|(i, (field, salt))| make_leaf(i as u8, salt, field))
        .collect();

    let root = make_root(&leaves);
    (root, leaves)
}

/// Verify a selective-disclosure proof against a known root.
///
/// The verifier supplies:
/// - `root` — the attested 32-byte commitment (from the chain).
/// - `disclosures` — the revealed `(index, salt, field)` tuples.
/// - `undisclosed_leaves` — the raw leaf hashes for all undisclosed fields,
///   in ascending index order.
///
/// The function reconstructs the full leaf list (disclosed + undisclosed),
/// places them in schema order by index, recomputes the root, and returns
/// `true` if and only if it matches.
///
/// # Security note
///
/// This function only confirms that the disclosed fields are consistent with
/// the attested root. The caller must **also** check that the root has a valid
/// attestation on-chain (via `attestation-registry.get_attestation`).
pub fn verify_disclosure(
    root: &[u8; 32],
    disclosures: &[Disclosure],
    undisclosed_leaves: &[[u8; 32]],
) -> bool {
    let n_disclosed = disclosures.len();
    let n_undisclosed = undisclosed_leaves.len();
    let total = n_disclosed + n_undisclosed;

    if total == 0 {
        return false;
    }
    if total > 256 {
        return false;
    }

    // Build a map of index → leaf for every position.
    let mut leaf_map: std::collections::BTreeMap<u8, [u8; 32]> = std::collections::BTreeMap::new();

    // Insert disclosed leaves (recomputed from the supplied data).
    for d in disclosures {
        let leaf = make_leaf(d.index, &d.salt, &d.field);
        if leaf_map.insert(d.index, leaf).is_some() {
            return false; // duplicate index
        }
    }

    // We need to assign indices to undisclosed leaves. The verifier must know
    // the total field count and the indices of undisclosed fields. Here we
    // derive them by filling the gaps in ascending order.
    let disclosed_indices: std::collections::BTreeSet<u8> =
        disclosures.iter().map(|d| d.index).collect();

    // Collect indices not present in the disclosed set, in order.
    let undisclosed_indices: Vec<u8> = (0u8..total as u8)
        .filter(|i| !disclosed_indices.contains(i))
        .collect();

    if undisclosed_indices.len() != n_undisclosed {
        return false;
    }

    for (idx, &leaf) in undisclosed_indices.iter().zip(undisclosed_leaves.iter()) {
        leaf_map.insert(*idx, leaf);
    }

    // Collect leaves in index order and recompute the root.
    let ordered: Vec<[u8; 32]> = leaf_map.into_values().collect();
    let recomputed = make_root(&ordered);
    recomputed == *root
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::{fs, path::Path};

    // ------------------------------------------------------------------
    // Shared fixture loading
    // ------------------------------------------------------------------

    #[derive(Debug, Deserialize)]
    struct Lrc2Vector {
        name: String,
        fields: Vec<JsonField>,
        salts_hex: Vec<String>,
        leaves_hex: Vec<String>,
        root_hex: String,
    }

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

    fn load_lrc2_vectors() -> Vec<Lrc2Vector> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/lrc2-test-vectors.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("read lrc2 test vector fixture at {:?}", path));
        serde_json::from_str(&raw).expect("parse lrc2 test vector fixture")
    }

    fn hex_to_32(s: &str) -> [u8; 32] {
        let v = hex::decode(s).expect("valid hex");
        v.try_into().expect("32 bytes")
    }

    // ------------------------------------------------------------------
    // Cross-language fixture test
    // ------------------------------------------------------------------

    #[test]
    fn matches_lrc2_cross_language_vectors() {
        let vectors = load_lrc2_vectors();
        assert!(!vectors.is_empty(), "lrc2 fixture must not be empty");

        for vector in vectors {
            let fields: Vec<FieldValue> =
                vector.fields.into_iter().map(FieldValue::from).collect();
            let salts: Vec<[u8; 32]> = vector
                .salts_hex
                .iter()
                .map(|s| hex_to_32(s))
                .collect();

            let (root, leaves) = commit_lrc2(&fields, &salts);

            for (i, (leaf, expected)) in leaves.iter().zip(vector.leaves_hex.iter()).enumerate() {
                assert_eq!(
                    hex::encode(leaf),
                    *expected,
                    "leaf[{i}] mismatch for vector `{}`",
                    vector.name
                );
            }

            assert_eq!(
                hex::encode(root),
                vector.root_hex,
                "root mismatch for vector `{}`",
                vector.name
            );
        }
    }

    // ------------------------------------------------------------------
    // Property tests
    // ------------------------------------------------------------------

    #[test]
    fn disclosure_verification_roundtrip() {
        let fields = vec![
            FieldValue::Text("O+".into()),
            FieldValue::Text("AS".into()),
            FieldValue::Text("penicillin - anaphylaxis".into()),
            FieldValue::Null,
        ];
        let salts: Vec<[u8; 32]> = (0..fields.len()).map(|i| [i as u8; 32]).collect();
        let (root, leaves) = commit_lrc2(&fields, &salts);

        // Disclose fields 0 and 2 (blood group and allergy); withhold 1 and 3.
        let disclosures = vec![
            Disclosure {
                index: 0,
                salt: salts[0],
                field: fields[0].clone(),
            },
            Disclosure {
                index: 2,
                salt: salts[2],
                field: fields[2].clone(),
            },
        ];
        let undisclosed = vec![leaves[1], leaves[3]];
        assert!(verify_disclosure(&root, &disclosures, &undisclosed));
    }

    #[test]
    fn disclosure_fails_with_wrong_root() {
        let fields = vec![FieldValue::Text("A+".into()), FieldValue::Bool(true)];
        let salts: Vec<[u8; 32]> = (0..fields.len()).map(|i| [i as u8; 32]).collect();
        let (root, leaves) = commit_lrc2(&fields, &salts);

        let wrong_root = [0xffu8; 32];
        assert_ne!(root, wrong_root);

        let disclosures = vec![Disclosure {
            index: 0,
            salt: salts[0],
            field: fields[0].clone(),
        }];
        let undisclosed = vec![leaves[1]];
        assert!(!verify_disclosure(&wrong_root, &disclosures, &undisclosed));
    }

    #[test]
    fn disclosure_fails_with_wrong_field_value() {
        let fields = vec![FieldValue::Text("O+".into()), FieldValue::Text("AB-".into())];
        let salts: Vec<[u8; 32]> = (0..fields.len()).map(|i| [i as u8; 32]).collect();
        let (root, leaves) = commit_lrc2(&fields, &salts);

        // Claim blood group is A+ instead of O+.
        let disclosures = vec![Disclosure {
            index: 0,
            salt: salts[0],
            field: FieldValue::Text("A+".into()), // wrong
        }];
        let undisclosed = vec![leaves[1]];
        assert!(!verify_disclosure(&root, &disclosures, &undisclosed));
    }

    #[test]
    fn different_salts_produce_different_leaves() {
        let field = FieldValue::Text("O+".into());
        let salt_a = [0xaau8; 32];
        let salt_b = [0xbbu8; 32];
        assert_ne!(make_leaf(0, &salt_a, &field), make_leaf(0, &salt_b, &field));
    }

    #[test]
    fn same_value_different_index_produces_different_leaf() {
        let field = FieldValue::Text("same".into());
        let salt = [0x42u8; 32];
        assert_ne!(make_leaf(0, &salt, &field), make_leaf(1, &salt, &field));
    }

    #[test]
    fn field_order_changes_root() {
        let f_a = FieldValue::Text("alpha".into());
        let f_b = FieldValue::Text("beta".into());
        let salt = [0u8; 32];

        let (root_ab, _) = commit_lrc2(&[f_a.clone(), f_b.clone()], &[[0u8; 32], [1u8; 32]]);
        let (root_ba, _) = commit_lrc2(&[f_b.clone(), f_a.clone()], &[[0u8; 32], [1u8; 32]]);
        assert_ne!(root_ab, root_ba);
        let _ = salt;
    }

    #[test]
    fn full_disclosure_verifies() {
        let fields = vec![FieldValue::Bool(false), FieldValue::Int64(0)];
        let salts: Vec<[u8; 32]> = vec![[0u8; 32], [1u8; 32]];
        let (root, _) = commit_lrc2(&fields, &salts);

        let disclosures: Vec<Disclosure> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| Disclosure {
                index: i as u8,
                salt: salts[i],
                field: f.clone(),
            })
            .collect();
        assert!(verify_disclosure(&root, &disclosures, &[]));
    }
}
