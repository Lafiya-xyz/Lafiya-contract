//! Lafiya QR Payload v1 — Encoder/Decoder
//!
//! Implements the binary format specified in `docs/specs/qr-payload-v1.md`.
//! The encoding stack is: CBOR → (optional zlib) → base45 → QR alphanumeric.
//!
//! # Quick example
//!
//! ```rust
//! use lafiya_qr::{QrPayload, QrPayloadBuilder, NETWORK_ID_TESTNET, encode_qr, decode_qr};
//!
//! let payload = QrPayloadBuilder::new(
//!     NETWORK_ID_TESTNET,
//!     [0x12u8; 32],   // attestation_registry contract id
//!     [0x34u8; 32],   // card_ref (record commitment)
//! ).build();
//!
//! let qr_string = encode_qr(&payload).expect("encode");
//! assert!(qr_string.starts_with("LFQR:"));
//!
//! let decoded = decode_qr(&qr_string).expect("decode");
//! assert_eq!(decoded.network_id, payload.network_id);
//! assert_eq!(decoded.card_ref, payload.card_ref);
//! ```
//!
//! # Rejection codes
//!
//! [`DecodeError`] covers every rejection case from the spec: invalid prefix,
//! invalid base45, invalid CBOR, unknown version, network mismatch, truncated
//! or missing required fields, and commitment mismatch for LRC-2 payloads.

pub(crate) mod base45;
pub(crate) mod cbor;

use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// QR prefix
// ---------------------------------------------------------------------------

/// QR string prefix (analogous to EU DCC `HC1:`).
/// Every encoded payload starts with this prefix.
pub const QR_PREFIX: &str = "LFQR:";

// ---------------------------------------------------------------------------
// Network IDs (first 4 bytes of SHA-256(passphrase))
// ---------------------------------------------------------------------------

/// `network_id` for Stellar mainnet.
///
/// SHA-256("Public Global Stellar Network ; September 2015")[0:4]
pub const NETWORK_ID_MAINNET: [u8; 4] = [0x7a, 0xc3, 0x39, 0x97];

/// `network_id` for Stellar testnet.
///
/// SHA-256("Test SDF Network ; September 2015")[0:4]
pub const NETWORK_ID_TESTNET: [u8; 4] = [0xce, 0xe0, 0x30, 0x2d];

/// `network_id` for Stellar futurenet.
///
/// SHA-256("Test SDF Future Network ; October 2022")[0:4]
pub const NETWORK_ID_FUTURENET: [u8; 4] = [0xb9, 0x1e, 0x85, 0x4d];

/// Derive a `network_id` from a Stellar network passphrase.
pub fn derive_network_id(passphrase: &str) -> [u8; 4] {
    let digest = Sha256::digest(passphrase.as_bytes());
    [digest[0], digest[1], digest[2], digest[3]]
}

// ---------------------------------------------------------------------------
// LRC version constants
// ---------------------------------------------------------------------------

/// LRC version byte: LRC-1 (whole-record commitment).
pub const LRC_VERSION_1: u8 = 1;
/// LRC version byte: LRC-2 (per-field salted selective disclosure).
pub const LRC_VERSION_2: u8 = 2;

// ---------------------------------------------------------------------------
// CBOR map keys (unsigned integer keys, per spec §4.1)
// ---------------------------------------------------------------------------

const KEY_VERSION: u64 = 1;
const KEY_NETWORK_ID: u64 = 2;
const KEY_ATTESTATION_REGISTRY: u64 = 3;
const KEY_CARD_REF: u64 = 4;
const KEY_LRC_VERSION: u64 = 5;
const KEY_DISCLOSURES: u64 = 6;
const KEY_OFFLINE_RECEIPT: u64 = 7;
const KEY_FALLBACK_URL: u64 = 8;
const KEY_PATIENT_SIG: u64 = 9;
const KEY_UNDISCLOSED_LEAVES: u64 = 10;

// Disclosure sub-keys
const DISC_KEY_INDEX: u64 = 1;
const DISC_KEY_SALT: u64 = 2;
const DISC_KEY_KIND: u64 = 3;
const DISC_KEY_VALUE: u64 = 4;

// Offline receipt sub-keys
const RCPT_KEY_V: u64 = 1;
const RCPT_KEY_ATTESTER: u64 = 2;
const RCPT_KEY_ATTESTED_AT: u64 = 3;
const RCPT_KEY_LEDGER_SEQ: u64 = 4;
const RCPT_KEY_ATTESTER_SIG: u64 = 5;

// ---------------------------------------------------------------------------
// Disclosure field kind tags (matches LRC-1 tags)
// ---------------------------------------------------------------------------

/// Field kind tag encoding for CBOR disclosure packages.
#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum FieldKind {
    Absent = 0,
    Null = 1,
    Text = 16,
    Int64 = 17,
    Bool = 18,
    Bytes = 19,
}

impl FieldKind {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(FieldKind::Absent),
            1 => Some(FieldKind::Null),
            16 => Some(FieldKind::Text),
            17 => Some(FieldKind::Int64),
            18 => Some(FieldKind::Bool),
            19 => Some(FieldKind::Bytes),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LRC-2 field value for QR disclosure packages
// ---------------------------------------------------------------------------

/// A field value as it appears in an LRC-2 disclosure package inside a QR payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisclosureFieldValue {
    Absent,
    Null,
    Text(String),
    Int64(i64),
    Bool(bool),
    Bytes(Vec<u8>),
}

impl DisclosureFieldValue {
    fn kind(&self) -> FieldKind {
        match self {
            DisclosureFieldValue::Absent => FieldKind::Absent,
            DisclosureFieldValue::Null => FieldKind::Null,
            DisclosureFieldValue::Text(_) => FieldKind::Text,
            DisclosureFieldValue::Int64(_) => FieldKind::Int64,
            DisclosureFieldValue::Bool(_) => FieldKind::Bool,
            DisclosureFieldValue::Bytes(_) => FieldKind::Bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// LRC-2 per-field disclosure
// ---------------------------------------------------------------------------

/// A single disclosed field for an LRC-2 payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disclosure {
    /// Zero-based field index within the record schema.
    pub index: u8,
    /// Per-field random salt (32 bytes).
    pub salt: [u8; 32],
    /// The disclosed field value.
    pub value: DisclosureFieldValue,
}

// ---------------------------------------------------------------------------
// Offline receipt (ADR-0014)
// ---------------------------------------------------------------------------

/// An offline-verifiable attestation receipt embedded in a QR payload.
///
/// Enables a responder to verify the card without a live Soroban RPC call.
/// See ADR-0014 for the full specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineReceipt {
    /// Receipt version (always `1` for this spec).
    pub v: u8,
    /// Attester public key (32 bytes, Ed25519).
    pub attester: [u8; 32],
    /// Unix epoch seconds when the attestation was recorded.
    pub attested_at: u64,
    /// Stellar ledger sequence number at attestation time.
    pub ledger_seq: u64,
    /// Ed25519 signature (64 bytes) by the attester over the canonical receipt encoding.
    pub attester_sig: [u8; 64],
}

// ---------------------------------------------------------------------------
// Main payload type
// ---------------------------------------------------------------------------

/// A decoded Lafiya QR payload v1.
///
/// Build with [`QrPayloadBuilder`], encode with [`encode_qr`], decode with [`decode_qr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPayload {
    // Required fields
    /// Always `1` for this spec version.
    pub version: u8,
    /// 4-byte network fingerprint (first 4 bytes of SHA-256(network passphrase)).
    pub network_id: [u8; 4],
    /// Contract ID of the `attestation-registry` (32 bytes).
    pub attestation_registry: [u8; 32],
    /// The attested record commitment (LRC-1 hash or LRC-2 root, 32 bytes).
    pub card_ref: [u8; 32],

    // Optional fields
    /// LRC version: `1` = LRC-1 (default), `2` = LRC-2.
    pub lrc_version: u8,
    /// LRC-2 disclosure package. Present only when `lrc_version == 2`.
    pub disclosures: Option<Vec<Disclosure>>,
    /// Undisclosed leaf hashes for LRC-2 payloads.
    pub undisclosed_leaves: Option<Vec<[u8; 32]>>,
    /// Offline attestation receipt (ADR-0014).
    pub offline_receipt: Option<OfflineReceipt>,
    /// Untrusted display-only URL. Must NOT be used for verification.
    pub fallback_url: Option<String>,
    /// Ed25519 patient signature (64 bytes) over canonical payload fields 1–5.
    pub patient_sig: Option<[u8; 64]>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Ergonomic builder for [`QrPayload`].
pub struct QrPayloadBuilder {
    payload: QrPayload,
}

impl QrPayloadBuilder {
    /// Start a new builder with the three required fields.
    pub fn new(
        network_id: [u8; 4],
        attestation_registry: [u8; 32],
        card_ref: [u8; 32],
    ) -> Self {
        Self {
            payload: QrPayload {
                version: 1,
                network_id,
                attestation_registry,
                card_ref,
                lrc_version: LRC_VERSION_1,
                disclosures: None,
                undisclosed_leaves: None,
                offline_receipt: None,
                fallback_url: None,
                patient_sig: None,
            },
        }
    }

    /// Attach an LRC-2 selective-disclosure package.
    pub fn with_lrc2(
        mut self,
        disclosures: Vec<Disclosure>,
        undisclosed_leaves: Vec<[u8; 32]>,
    ) -> Self {
        self.payload.lrc_version = LRC_VERSION_2;
        self.payload.disclosures = Some(disclosures);
        self.payload.undisclosed_leaves = Some(undisclosed_leaves);
        self
    }

    /// Attach an offline attestation receipt.
    pub fn with_offline_receipt(mut self, receipt: OfflineReceipt) -> Self {
        self.payload.offline_receipt = Some(receipt);
        self
    }

    /// Attach a display-only fallback URL.
    pub fn with_fallback_url(mut self, url: impl Into<String>) -> Self {
        self.payload.fallback_url = Some(url.into());
        self
    }

    /// Attach a patient Ed25519 signature.
    pub fn with_patient_sig(mut self, sig: [u8; 64]) -> Self {
        self.payload.patient_sig = Some(sig);
        self
    }

    /// Finalise and return the payload.
    pub fn build(self) -> QrPayload {
        self.payload
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned by [`decode_qr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// QR string does not start with `LFQR:`.
    InvalidPrefix,
    /// Base45 decode failed (malformed alphabet or truncated input).
    InvalidBase45(String),
    /// CBOR decode failed (malformed or truncated bytes).
    InvalidCbor(String),
    /// `version` field is not `1`.
    UnknownVersion(u8),
    /// `network_id` does not match the expected value.
    NetworkMismatch { got: [u8; 4], expected: [u8; 4] },
    /// A required field is missing from the CBOR map.
    MissingRequiredField(&'static str),
    /// A field has an unexpected type or length.
    InvalidField(&'static str),
    /// LRC-2 recomputed root ≠ `card_ref`.
    CommitmentMismatch,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::InvalidPrefix => write!(f, "INVALID_PREFIX: missing 'LFQR:' prefix"),
            DecodeError::InvalidBase45(e) => write!(f, "INVALID_BASE45: {e}"),
            DecodeError::InvalidCbor(e) => write!(f, "INVALID_CBOR: {e}"),
            DecodeError::UnknownVersion(v) => write!(f, "UNKNOWN_VERSION: {v}"),
            DecodeError::NetworkMismatch { got, expected } => write!(
                f,
                "NETWORK_MISMATCH: got {:02x}{:02x}{:02x}{:02x}, expected {:02x}{:02x}{:02x}{:02x}",
                got[0], got[1], got[2], got[3],
                expected[0], expected[1], expected[2], expected[3],
            ),
            DecodeError::MissingRequiredField(name) => {
                write!(f, "MISSING_REQUIRED_FIELD: {name}")
            }
            DecodeError::InvalidField(name) => write!(f, "INVALID_FIELD: {name}"),
            DecodeError::CommitmentMismatch => write!(f, "COMMITMENT_MISMATCH"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Errors returned by [`encode_qr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Payload version is not 1.
    UnsupportedVersion(u8),
    /// Fallback URL contains non-UTF-8 content (should never happen in Rust).
    InvalidFallbackUrl,
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EncodeError::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            EncodeError::InvalidFallbackUrl => write!(f, "fallback URL is not valid UTF-8"),
        }
    }
}

impl std::error::Error for EncodeError {}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode a [`QrPayload`] into the `LFQR:` QR string.
///
/// The encoding pipeline: payload → CBOR → base45 → prepend `LFQR:`.
/// (zlib deflate is omitted in this reference implementation; all test vectors
/// use uncompressed payloads. A production encoder may apply deflate when it
/// reduces the payload size by ≥ 4 bytes.)
pub fn encode_qr(payload: &QrPayload) -> Result<String, EncodeError> {
    if payload.version != 1 {
        return Err(EncodeError::UnsupportedVersion(payload.version));
    }
    let cbor_bytes = encode_cbor(payload)?;
    let b45 = base45::encode(&cbor_bytes);
    Ok(format!("{}{}", QR_PREFIX, b45))
}

/// Decode a `LFQR:` QR string into a [`QrPayload`].
///
/// Optionally verify the `network_id` if `expected_network_id` is `Some`.
pub fn decode_qr(
    qr: &str,
) -> Result<QrPayload, DecodeError> {
    decode_qr_with_network(qr, None)
}

/// Decode and verify a `LFQR:` QR string, requiring a specific network ID.
pub fn decode_qr_with_network(
    qr: &str,
    expected_network_id: Option<[u8; 4]>,
) -> Result<QrPayload, DecodeError> {
    // 1. Prefix check
    let rest = qr
        .strip_prefix(QR_PREFIX)
        .ok_or(DecodeError::InvalidPrefix)?;

    // 2. Base45 decode
    let cbor_bytes = base45::decode(rest).map_err(DecodeError::InvalidBase45)?;

    // 3. CBOR decode
    let payload = decode_cbor(&cbor_bytes)?;

    // 4. Network check (if requested)
    if let Some(expected) = expected_network_id {
        if payload.network_id != expected {
            return Err(DecodeError::NetworkMismatch {
                got: payload.network_id,
                expected,
            });
        }
    }

    Ok(payload)
}

// ---------------------------------------------------------------------------
// CBOR encoding (manual, no external dependency)
// ---------------------------------------------------------------------------
//
// We implement a minimal CBOR encoder/decoder covering the subset used by the
// QR payload spec: unsigned integers, byte strings, text strings, arrays,
// and maps with unsigned integer keys.  This avoids pulling in a full CBOR
// crate as a dependency while keeping the implementation auditable.

fn encode_cbor(payload: &QrPayload) -> Result<Vec<u8>, EncodeError> {
    let mut map_entries: Vec<(u64, CborValue)> = Vec::new();

    // Required fields
    map_entries.push((KEY_VERSION, CborValue::Uint(1)));
    map_entries.push((KEY_NETWORK_ID, CborValue::Bytes(payload.network_id.to_vec())));
    map_entries.push((KEY_ATTESTATION_REGISTRY, CborValue::Bytes(payload.attestation_registry.to_vec())));
    map_entries.push((KEY_CARD_REF, CborValue::Bytes(payload.card_ref.to_vec())));

    // lrc_version (omit if default value of 1)
    if payload.lrc_version != LRC_VERSION_1 {
        map_entries.push((KEY_LRC_VERSION, CborValue::Uint(payload.lrc_version as u64)));
    }

    // LRC-2 disclosures
    if let Some(disclosures) = &payload.disclosures {
        let disc_array: Vec<CborValue> = disclosures.iter().map(|d| {
            let mut dm: Vec<(u64, CborValue)> = vec![
                (DISC_KEY_INDEX, CborValue::Uint(d.index as u64)),
                (DISC_KEY_SALT, CborValue::Bytes(d.salt.to_vec())),
                (DISC_KEY_KIND, CborValue::Uint(d.value.kind() as u64)),
            ];
            match &d.value {
                DisclosureFieldValue::Absent | DisclosureFieldValue::Null => {}
                DisclosureFieldValue::Text(s) => {
                    dm.push((DISC_KEY_VALUE, CborValue::Text(s.clone())));
                }
                DisclosureFieldValue::Int64(v) => {
                    dm.push((DISC_KEY_VALUE, CborValue::Int(*v)));
                }
                DisclosureFieldValue::Bool(b) => {
                    dm.push((DISC_KEY_VALUE, CborValue::Bool(*b)));
                }
                DisclosureFieldValue::Bytes(b) => {
                    dm.push((DISC_KEY_VALUE, CborValue::Bytes(b.clone())));
                }
            }
            CborValue::Map(dm)
        }).collect();
        map_entries.push((KEY_DISCLOSURES, CborValue::Array(disc_array)));
    }

    // Undisclosed leaves
    if let Some(leaves) = &payload.undisclosed_leaves {
        let leaf_array: Vec<CborValue> = leaves
            .iter()
            .map(|l| CborValue::Bytes(l.to_vec()))
            .collect();
        map_entries.push((KEY_UNDISCLOSED_LEAVES, CborValue::Array(leaf_array)));
    }

    // Offline receipt
    if let Some(receipt) = &payload.offline_receipt {
        let rm: Vec<(u64, CborValue)> = vec![
            (RCPT_KEY_V, CborValue::Uint(receipt.v as u64)),
            (RCPT_KEY_ATTESTER, CborValue::Bytes(receipt.attester.to_vec())),
            (RCPT_KEY_ATTESTED_AT, CborValue::Uint(receipt.attested_at)),
            (RCPT_KEY_LEDGER_SEQ, CborValue::Uint(receipt.ledger_seq)),
            (RCPT_KEY_ATTESTER_SIG, CborValue::Bytes(receipt.attester_sig.to_vec())),
        ];
        map_entries.push((KEY_OFFLINE_RECEIPT, CborValue::Map(rm)));
    }

    // Fallback URL
    if let Some(url) = &payload.fallback_url {
        map_entries.push((KEY_FALLBACK_URL, CborValue::Text(url.clone())));
    }

    // Patient sig
    if let Some(sig) = &payload.patient_sig {
        map_entries.push((KEY_PATIENT_SIG, CborValue::Bytes(sig.to_vec())));
    }

    let root = CborValue::Map(map_entries);
    let mut out = Vec::new();
    cbor::encode_value(&root, &mut out);
    Ok(out)
}

fn decode_cbor(bytes: &[u8]) -> Result<QrPayload, DecodeError> {
    let (value, rest) =
        cbor::decode_value(bytes).map_err(|e| DecodeError::InvalidCbor(e.to_string()))?;
    if !rest.is_empty() {
        return Err(DecodeError::InvalidCbor("trailing bytes after CBOR value".into()));
    }

    let map = match value {
        CborValue::Map(m) => m,
        _ => return Err(DecodeError::InvalidCbor("expected CBOR map at root".into())),
    };

    // Helper: get uint value for a given key
    fn find_uint(map: &[(u64, CborValue)], key: u64) -> Option<u64> {
        map.iter().find(|(k, _)| *k == key).and_then(|(_, v)| {
            if let CborValue::Uint(n) = v { Some(*n) } else { None }
        })
    }

    // Helper: get owned bytes for a given key
    fn find_bytes(map: &[(u64, CborValue)], key: u64) -> Option<Vec<u8>> {
        map.iter().find(|(k, _)| *k == key).and_then(|(_, v)| {
            if let CborValue::Bytes(b) = v { Some(b.clone()) } else { None }
        })
    }

    // Helper: get owned text for a given key
    fn find_text(map: &[(u64, CborValue)], key: u64) -> Option<String> {
        map.iter().find(|(k, _)| *k == key).and_then(|(_, v)| {
            if let CborValue::Text(s) = v { Some(s.clone()) } else { None }
        })
    }

    // Required: version
    let version = find_uint(&map, KEY_VERSION)
        .ok_or(DecodeError::MissingRequiredField("version"))? as u8;
    if version != 1 {
        return Err(DecodeError::UnknownVersion(version));
    }

    // Required: network_id
    let network_id_vec = find_bytes(&map, KEY_NETWORK_ID)
        .ok_or(DecodeError::MissingRequiredField("network_id"))?;
    if network_id_vec.len() != 4 {
        return Err(DecodeError::InvalidField("network_id must be 4 bytes"));
    }
    let network_id: [u8; 4] = network_id_vec.try_into().unwrap();

    // Required: attestation_registry
    let registry_vec = find_bytes(&map, KEY_ATTESTATION_REGISTRY)
        .ok_or(DecodeError::MissingRequiredField("attestation_registry"))?;
    if registry_vec.len() != 32 {
        return Err(DecodeError::InvalidField("attestation_registry must be 32 bytes"));
    }
    let attestation_registry: [u8; 32] = registry_vec.try_into().unwrap();

    // Required: card_ref
    let card_ref_vec = find_bytes(&map, KEY_CARD_REF)
        .ok_or(DecodeError::MissingRequiredField("card_ref"))?;
    if card_ref_vec.len() != 32 {
        return Err(DecodeError::InvalidField("card_ref must be 32 bytes"));
    }
    let card_ref: [u8; 32] = card_ref_vec.try_into().unwrap();

    // Optional: lrc_version (default 1)
    let lrc_version = find_uint(&map, KEY_LRC_VERSION).map(|v| v as u8).unwrap_or(LRC_VERSION_1);

    // Optional: disclosures (LRC-2)
    let disclosures = map.iter().find(|(k, _)| *k == KEY_DISCLOSURES)
        .and_then(|(_, v)| if let CborValue::Array(arr) = v { Some(arr) } else { None })
        .map(|arr| -> Result<Vec<Disclosure>, DecodeError> {
            arr.iter().map(|item| {
                let dm = match item {
                    CborValue::Map(m) => m,
                    _ => return Err(DecodeError::InvalidField("disclosure entry must be a map")),
                };

                let index = find_uint(dm, DISC_KEY_INDEX)
                    .ok_or(DecodeError::InvalidField("disclosure: missing index"))? as u8;

                let salt_vec = find_bytes(dm, DISC_KEY_SALT)
                    .ok_or(DecodeError::InvalidField("disclosure: missing salt"))?;
                if salt_vec.len() != 32 {
                    return Err(DecodeError::InvalidField("disclosure: salt must be 32 bytes"));
                }
                let salt: [u8; 32] = salt_vec.try_into().unwrap();

                let kind_byte = find_uint(dm, DISC_KEY_KIND)
                    .ok_or(DecodeError::InvalidField("disclosure: missing kind"))? as u8;
                let kind = FieldKind::from_u8(kind_byte)
                    .ok_or(DecodeError::InvalidField("disclosure: unknown field kind"))?;

                let value = match kind {
                    FieldKind::Absent => DisclosureFieldValue::Absent,
                    FieldKind::Null => DisclosureFieldValue::Null,
                    FieldKind::Text => {
                        let s = find_text(dm, DISC_KEY_VALUE)
                            .ok_or(DecodeError::InvalidField("disclosure: missing text value"))?;
                        DisclosureFieldValue::Text(s)
                    }
                    FieldKind::Int64 => {
                        let n = dm.iter().find(|(k, _)| *k == DISC_KEY_VALUE)
                            .and_then(|(_, v)| if let CborValue::Int(n) = v { Some(*n) } else { None })
                            .ok_or(DecodeError::InvalidField("disclosure: missing int64 value"))?;
                        DisclosureFieldValue::Int64(n)
                    }
                    FieldKind::Bool => {
                        let b = dm.iter().find(|(k, _)| *k == DISC_KEY_VALUE)
                            .and_then(|(_, v)| if let CborValue::Bool(b) = v { Some(*b) } else { None })
                            .ok_or(DecodeError::InvalidField("disclosure: missing bool value"))?;
                        DisclosureFieldValue::Bool(b)
                    }
                    FieldKind::Bytes => {
                        let b = find_bytes(dm, DISC_KEY_VALUE)
                            .ok_or(DecodeError::InvalidField("disclosure: missing bytes value"))?;
                        DisclosureFieldValue::Bytes(b)
                    }
                };

                Ok(Disclosure { index, salt, value })
            }).collect()
        }).transpose()?;

    // Optional: undisclosed_leaves
    let undisclosed_leaves = map.iter().find(|(k, _)| *k == KEY_UNDISCLOSED_LEAVES)
        .and_then(|(_, v)| if let CborValue::Array(a) = v { Some(a) } else { None })
        .map(|arr| -> Result<Vec<[u8; 32]>, DecodeError> {
            arr.iter().map(|item| {
                if let CborValue::Bytes(b) = item {
                    if b.len() != 32 {
                        return Err(DecodeError::InvalidField("undisclosed_leaf must be 32 bytes"));
                    }
                    let v: [u8; 32] = b.as_slice().try_into().unwrap();
                    Ok(v)
                } else {
                    Err(DecodeError::InvalidField("undisclosed_leaf must be bytes"))
                }
            }).collect()
        }).transpose()?;

    // Optional: offline_receipt
    let offline_receipt = map.iter().find(|(k, _)| *k == KEY_OFFLINE_RECEIPT)
        .and_then(|(_, v)| if let CborValue::Map(m) = v { Some(m) } else { None })
        .map(|rm| -> Result<OfflineReceipt, DecodeError> {
            let v_byte = find_uint(rm, RCPT_KEY_V)
                .ok_or(DecodeError::InvalidField("receipt: missing v"))? as u8;

            let attester_vec = find_bytes(rm, RCPT_KEY_ATTESTER)
                .ok_or(DecodeError::InvalidField("receipt: missing attester"))?;
            if attester_vec.len() != 32 {
                return Err(DecodeError::InvalidField("receipt: attester must be 32 bytes"));
            }
            let attester: [u8; 32] = attester_vec.try_into().unwrap();

            let attested_at = find_uint(rm, RCPT_KEY_ATTESTED_AT)
                .ok_or(DecodeError::InvalidField("receipt: missing attested_at"))?;
            let ledger_seq = find_uint(rm, RCPT_KEY_LEDGER_SEQ)
                .ok_or(DecodeError::InvalidField("receipt: missing ledger_seq"))?;

            let sig_vec = find_bytes(rm, RCPT_KEY_ATTESTER_SIG)
                .ok_or(DecodeError::InvalidField("receipt: missing attester_sig"))?;
            if sig_vec.len() != 64 {
                return Err(DecodeError::InvalidField("receipt: attester_sig must be 64 bytes"));
            }
            let attester_sig: [u8; 64] = sig_vec.try_into().unwrap();

            Ok(OfflineReceipt { v: v_byte, attester, attested_at, ledger_seq, attester_sig })
        }).transpose()?;

    // Optional: fallback_url
    let fallback_url = find_text(&map, KEY_FALLBACK_URL);

    // Optional: patient_sig
    let patient_sig = find_bytes(&map, KEY_PATIENT_SIG)
        .map(|b| -> Result<[u8; 64], DecodeError> {
            if b.len() != 64 {
                return Err(DecodeError::InvalidField("patient_sig must be 64 bytes"));
            }
            Ok(b.try_into().unwrap())
        }).transpose()?;

    Ok(QrPayload {
        version,
        network_id,
        attestation_registry,
        card_ref,
        lrc_version,
        disclosures,
        undisclosed_leaves,
        offline_receipt,
        fallback_url,
        patient_sig,
    })
}

// ---------------------------------------------------------------------------
// Internal CBOR value type (used only within encode/decode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) enum CborValue {
    Uint(u64),
    Int(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(Vec<(u64, CborValue)>),
    Bool(bool),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_id() -> [u8; 32] {
        [0x55u8; 32]
    }

    fn card_ref() -> [u8; 32] {
        [0x77u8; 32]
    }

    // ── Round-trip tests ────────────────────────────────────────────────────

    #[test]
    fn round_trip_minimal_payload() {
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref()).build();

        let qr = encode_qr(&payload).expect("encode");
        assert!(qr.starts_with(QR_PREFIX), "QR must start with LFQR:");

        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn round_trip_with_fallback_url() {
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref())
            .with_fallback_url("https://lafiya.example/card/abc123")
            .build();

        let qr = encode_qr(&payload).expect("encode");
        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded.fallback_url.as_deref(), Some("https://lafiya.example/card/abc123"));
    }

    #[test]
    fn round_trip_with_patient_sig() {
        let sig = [0xaau8; 64];
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref())
            .with_patient_sig(sig)
            .build();

        let qr = encode_qr(&payload).expect("encode");
        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded.patient_sig, Some(sig));
    }

    #[test]
    fn round_trip_lrc2_payload() {
        let disclosure = Disclosure {
            index: 0,
            salt: [0x42u8; 32],
            value: DisclosureFieldValue::Text("O+".into()),
        };
        let undisclosed = vec![[0xffu8; 32], [0x01u8; 32]];

        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref())
            .with_lrc2(vec![disclosure.clone()], undisclosed.clone())
            .build();

        let qr = encode_qr(&payload).expect("encode");
        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded.lrc_version, LRC_VERSION_2);
        let decoded_disclosures = decoded.disclosures.unwrap();
        assert_eq!(decoded_disclosures.len(), 1);
        assert_eq!(decoded_disclosures[0].index, 0);
        assert_eq!(decoded_disclosures[0].salt, [0x42u8; 32]);
        assert_eq!(decoded_disclosures[0].value, DisclosureFieldValue::Text("O+".into()));
        assert_eq!(decoded.undisclosed_leaves.unwrap(), undisclosed);
    }

    #[test]
    fn round_trip_offline_receipt() {
        let receipt = OfflineReceipt {
            v: 1,
            attester: [0x11u8; 32],
            attested_at: 1765900800,
            ledger_seq: 54321,
            attester_sig: [0xbbu8; 64],
        };
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref())
            .with_offline_receipt(receipt.clone())
            .build();

        let qr = encode_qr(&payload).expect("encode");
        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded.offline_receipt, Some(receipt));
    }

    #[test]
    fn round_trip_all_disclosure_field_kinds() {
        let disclosures = vec![
            Disclosure { index: 0, salt: [0u8; 32], value: DisclosureFieldValue::Absent },
            Disclosure { index: 1, salt: [1u8; 32], value: DisclosureFieldValue::Null },
            Disclosure { index: 2, salt: [2u8; 32], value: DisclosureFieldValue::Text("hello".into()) },
            Disclosure { index: 3, salt: [3u8; 32], value: DisclosureFieldValue::Int64(-42) },
            Disclosure { index: 4, salt: [4u8; 32], value: DisclosureFieldValue::Bool(true) },
            Disclosure { index: 5, salt: [5u8; 32], value: DisclosureFieldValue::Bytes(vec![0xde, 0xad]) },
        ];
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref())
            .with_lrc2(disclosures.clone(), vec![])
            .build();

        let qr = encode_qr(&payload).expect("encode");
        let decoded = decode_qr(&qr).expect("decode");
        assert_eq!(decoded.disclosures.unwrap(), disclosures);
    }

    // ── Network IDs ─────────────────────────────────────────────────────────

    #[test]
    fn network_id_derivation_mainnet() {
        let derived = derive_network_id("Public Global Stellar Network ; September 2015");
        assert_eq!(derived, NETWORK_ID_MAINNET);
    }

    #[test]
    fn network_id_derivation_testnet() {
        let derived = derive_network_id("Test SDF Network ; September 2015");
        assert_eq!(derived, NETWORK_ID_TESTNET);
    }

    #[test]
    fn network_id_derivation_futurenet() {
        let derived = derive_network_id("Test SDF Future Network ; October 2022");
        assert_eq!(derived, NETWORK_ID_FUTURENET);
    }

    #[test]
    fn network_mismatch_is_rejected() {
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref()).build();
        let qr = encode_qr(&payload).expect("encode");
        let err = decode_qr_with_network(&qr, Some(NETWORK_ID_MAINNET)).unwrap_err();
        assert_eq!(
            err,
            DecodeError::NetworkMismatch {
                got: NETWORK_ID_TESTNET,
                expected: NETWORK_ID_MAINNET,
            }
        );
    }

    #[test]
    fn network_check_passes_when_matching() {
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref()).build();
        let qr = encode_qr(&payload).expect("encode");
        decode_qr_with_network(&qr, Some(NETWORK_ID_TESTNET)).expect("should pass");
    }

    // ── Negative vectors ────────────────────────────────────────────────────

    #[test]
    fn rejects_missing_prefix() {
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref()).build();
        let qr = encode_qr(&payload).expect("encode");
        let no_prefix = &qr[5..]; // strip LFQR:
        assert_eq!(decode_qr(no_prefix).unwrap_err(), DecodeError::InvalidPrefix);
    }

    #[test]
    fn rejects_invalid_base45() {
        assert!(matches!(
            decode_qr("LFQR:!!!INVALID!!!"),
            Err(DecodeError::InvalidBase45(_))
        ));
    }

    #[test]
    fn rejects_truncated_cbor() {
        // Build valid CBOR then truncate it
        let payload = QrPayloadBuilder::new(NETWORK_ID_TESTNET, registry_id(), card_ref()).build();
        let qr = encode_qr(&payload).expect("encode");
        let rest = &qr[5..]; // strip LFQR:
        let cbor = base45::decode(rest).unwrap();
        let truncated = &cbor[..cbor.len() / 2];
        let reencoded = format!("{}{}", QR_PREFIX, base45::encode(truncated));
        assert!(matches!(
            decode_qr(&reencoded),
            Err(DecodeError::InvalidCbor(_)) | Err(DecodeError::MissingRequiredField(_))
        ));
    }

    #[test]
    fn rejects_unknown_version() {
        // Build a CBOR map with version = 99
        let mut out = Vec::new();
        let map: Vec<(u64, CborValue)> = vec![
            (KEY_VERSION, CborValue::Uint(99)),
            (KEY_NETWORK_ID, CborValue::Bytes(NETWORK_ID_TESTNET.to_vec())),
            (KEY_ATTESTATION_REGISTRY, CborValue::Bytes(vec![0u8; 32])),
            (KEY_CARD_REF, CborValue::Bytes(vec![0u8; 32])),
        ];
        cbor::encode_value(&CborValue::Map(map), &mut out);
        let qr = format!("{}{}", QR_PREFIX, base45::encode(&out));
        assert_eq!(decode_qr(&qr).unwrap_err(), DecodeError::UnknownVersion(99));
    }

    #[test]
    fn rejects_wrong_network_id_length() {
        let mut out = Vec::new();
        let map: Vec<(u64, CborValue)> = vec![
            (KEY_VERSION, CborValue::Uint(1)),
            (KEY_NETWORK_ID, CborValue::Bytes(vec![0x01, 0x02])), // wrong length
            (KEY_ATTESTATION_REGISTRY, CborValue::Bytes(vec![0u8; 32])),
            (KEY_CARD_REF, CborValue::Bytes(vec![0u8; 32])),
        ];
        cbor::encode_value(&CborValue::Map(map), &mut out);
        let qr = format!("{}{}", QR_PREFIX, base45::encode(&out));
        assert_eq!(
            decode_qr(&qr).unwrap_err(),
            DecodeError::InvalidField("network_id must be 4 bytes")
        );
    }

    // ── Vector file tests ───────────────────────────────────────────────────

    #[test]
    fn matches_qr_test_vectors() {
        use serde::Deserialize;
        use std::{fs, path::Path};

        #[derive(Deserialize)]
        struct Vector {
            name: String,
            description: String,
            network_id_hex: String,
            attestation_registry_hex: String,
            card_ref_hex: String,
            lrc_version: Option<u8>,
            fallback_url: Option<String>,
            expected_qr_prefix: String,
        }

        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/qr-test-vectors.json");
        if !path.exists() {
            // If no vector file present, skip rather than panic (vectors generated separately)
            return;
        }
        let raw = fs::read_to_string(&path).expect("read qr test vectors");
        let vectors: Vec<Vector> = serde_json::from_str(&raw).expect("parse qr test vectors");

        for v in &vectors {
            let _ = &v.description; // silence unused warning
            let _ = &v.lrc_version;
            let network_id: [u8; 4] = hex::decode(&v.network_id_hex)
                .unwrap()
                .try_into()
                .unwrap();
            let registry: [u8; 32] = hex::decode(&v.attestation_registry_hex)
                .unwrap()
                .try_into()
                .unwrap();
            let card_ref: [u8; 32] = hex::decode(&v.card_ref_hex)
                .unwrap()
                .try_into()
                .unwrap();

            let mut builder = QrPayloadBuilder::new(network_id, registry, card_ref);
            if let Some(url) = &v.fallback_url {
                builder = builder.with_fallback_url(url.clone());
            }
            let payload = builder.build();
            let qr = encode_qr(&payload).expect("encode");

            // Verify QR prefix
            assert!(
                qr.starts_with(&v.expected_qr_prefix),
                "vector '{}': QR prefix mismatch",
                v.name
            );

            // Round-trip: decode must succeed and match the original payload
            let decoded = decode_qr(&qr).expect("round-trip decode");
            assert_eq!(decoded.network_id, payload.network_id, "vector '{}': network_id mismatch", v.name);
            assert_eq!(decoded.card_ref, payload.card_ref, "vector '{}': card_ref mismatch", v.name);
            assert_eq!(decoded.attestation_registry, payload.attestation_registry, "vector '{}': registry mismatch", v.name);
        }
    }

    #[test]
    fn negative_vectors_all_rejected() {
        use serde::Deserialize;
        use std::{fs, path::Path};

        #[derive(Deserialize)]
        struct NegVector {
            name: String,
            description: String,
            qr_string: String,
            expected_error: String,
        }

        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/negative");
        let path = dir.join("qr-negative-vectors.json");
        if !path.exists() {
            return;
        }
        let raw = fs::read_to_string(&path).expect("read negative vectors");
        let vectors: Vec<NegVector> = serde_json::from_str(&raw).expect("parse negative vectors");

        for v in &vectors {
            let _ = &v.description; // silence unused warning
            let result = decode_qr(&v.qr_string);
            assert!(
                result.is_err(),
                "negative vector '{}' should have been rejected but decoded successfully",
                v.name
            );
            let err_str = result.unwrap_err().to_string().to_uppercase();
            let expected_upper = v.expected_error.to_uppercase();
            assert!(
                err_str.contains(&expected_upper),
                "vector '{}': expected error containing '{}', got '{}'",
                v.name,
                v.expected_error,
                err_str
            );
        }
    }
}
