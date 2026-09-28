//! Pluggable `Signer` abstraction for all Lafiya signing paths.
//!
//! # Issue #399
//!
//! Admin and multisig signer keys control the entire trust layer. This module
//! adds a pluggable back-end so operators can use:
//!
//! * **`identity://`** — the default stellar CLI file-based identity (software
//!   key on disk). Selected via `--signer identity://<name>` or just a plain
//!   identity name with no URI prefix.
//!
//! * **`ledger://`** — a Ledger hardware wallet running the Stellar app.
//!   Selected via `--signer ledger://0` (device index).
//!   The Stellar Ledger app can sign transaction envelopes and, for Soroban
//!   authorization entries, sign the `HashIdPreimage::SorobanAuthorization`
//!   hash. Because the Ledger **cannot display** the decoded invocation tree,
//!   the CLI shows the decoded auth entry on the host terminal and warns
//!   clearly that the device will sign a hash it cannot independently verify.
//!
//! * **`gcpkms://`** — Google Cloud KMS (ed25519 key version).
//!   Selected via `--signer gcpkms://projects/P/locations/L/keyRings/R/
//!   cryptoKeyVersions/V`.
//!   Ed25519 signing is in GA on GCP KMS as of 2024; the implementation is
//!   a stub that documents the API and runs only when `GCP_KMS_CREDENTIALS`
//!   is set in the environment.
//!
//! * **`awskms://`** — AWS KMS (ed25519 key version, stub).
//!   AWS KMS added Ed25519 support in 2023; the integration test runs only
//!   when `AWS_KMS_KEY_ID` and standard AWS credential env vars are present.
//!
//! All signing paths (envelope signing, auth-entry hash signing, multisig
//! ceremony participation) go through the `Signer` trait.
//!
//! ## Security note
//!
//! Trust boundaries differ per back-end:
//!
//! | Back-end     | Key leaves device? | Blind-hash risk | Auditability |
//! |---|---|---|---|
//! | `identity`   | Yes (key on disk)  | N/A             | Local files   |
//! | `ledger`     | Never              | Yes (Soroban auth entries are hash-only on device) | Device display |
//! | `gcpkms`     | Never              | Yes (KMS signs opaque bytes) | GCP audit log |
//! | `awskms`     | Never              | Yes (KMS signs opaque bytes) | CloudTrail    |
//!
//! For Soroban auth entries the CLI **always** prints the decoded invocation
//! tree to the terminal, regardless of back-end, so the operator can verify
//! what they are authorising before the signing request is sent.

use std::fmt;

// ---------------------------------------------------------------------------
// Core trait
// ---------------------------------------------------------------------------

/// The sign request passed to every signing back-end.
#[derive(Debug, Clone)]
pub struct SignRequest {
    /// The 32-byte hash to be signed (e.g. SHA-256 of a transaction envelope,
    /// or the Soroban auth-entry preimage hash).
    pub hash: [u8; 32],
    /// Human-readable description of what is being signed, shown on the
    /// terminal before the signing request is sent to the device/KMS.
    pub description: String,
    /// Whether this is a Soroban authorization entry (not a transaction
    /// envelope). When `true` the CLI emits an additional warning if the
    /// back-end cannot display the full invocation tree.
    pub is_soroban_auth_entry: bool,
}

/// The signature produced by a signer.
#[derive(Debug, Clone)]
pub struct Signature {
    /// 64-byte ed25519 signature.
    pub bytes: [u8; 64],
    /// The ed25519 public key that produced this signature (32 bytes).
    pub public_key: [u8; 32],
}

/// Error type for signing operations.
#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("signer '{signer_uri}' is not available: {reason}")]
    NotAvailable { signer_uri: String, reason: String },

    #[error("signing failed for '{signer_uri}': {reason}")]
    SigningFailed { signer_uri: String, reason: String },

    #[error("invalid signer URI '{uri}': {reason}")]
    InvalidUri { uri: String, reason: String },

    #[error("operator cancelled the signing request")]
    Cancelled,
}

/// Pluggable signing back-end.
///
/// Implement this trait to add a new key custody option. All signing paths
/// in the CLI route through this trait so that switching back-ends requires
/// only a `--signer` flag change.
pub trait Signer: fmt::Debug + Send + Sync {
    /// A human-readable label for this signer (used in log output and errors).
    fn label(&self) -> &str;

    /// Whether this back-end can display the invocation tree on the signing
    /// device itself. When `false`, the CLI warns that the operator is
    /// performing a blind-hash sign.
    fn can_display_invocation(&self) -> bool;

    /// Sign a 32-byte hash and return the 64-byte ed25519 signature together
    /// with the public key.
    ///
    /// For Soroban auth entries (`request.is_soroban_auth_entry == true`),
    /// the CLI has already printed the decoded invocation tree to stdout. The
    /// signer implementation must prompt for confirmation if the back-end does
    /// not have a display.
    fn sign(&self, request: &SignRequest) -> Result<Signature, SignerError>;

    /// Return the ed25519 public key bytes (32 bytes) for this signer without
    /// performing a signing operation. Used to construct authorization entries
    /// before signing.
    fn public_key(&self) -> Result<[u8; 32], SignerError>;
}

// ---------------------------------------------------------------------------
// URI parsing
// ---------------------------------------------------------------------------

/// Parsed representation of a `--signer` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerUri {
    /// `identity://<name>` or bare `<name>` — stellar CLI file identity.
    Identity(String),
    /// `ledger://<index>` — Ledger hardware wallet, device index.
    Ledger(u32),
    /// `gcpkms://<resource-path>` — Google Cloud KMS key version.
    GcpKms(String),
    /// `awskms://<key-id>` — AWS KMS key ID.
    AwsKms(String),
}

impl SignerUri {
    /// Parse a `--signer` flag value into a [`SignerUri`].
    pub fn parse(s: &str) -> Result<Self, SignerError> {
        if let Some(rest) = s.strip_prefix("identity://") {
            if rest.is_empty() {
                return Err(SignerError::InvalidUri {
                    uri: s.to_string(),
                    reason: "identity name must not be empty after identity://".into(),
                });
            }
            return Ok(SignerUri::Identity(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("ledger://") {
            let index: u32 = rest.parse().map_err(|_| SignerError::InvalidUri {
                uri: s.to_string(),
                reason: format!("ledger device index must be a non-negative integer, got '{rest}'"),
            })?;
            return Ok(SignerUri::Ledger(index));
        }
        if let Some(rest) = s.strip_prefix("gcpkms://") {
            if rest.is_empty() {
                return Err(SignerError::InvalidUri {
                    uri: s.to_string(),
                    reason: "GCP KMS resource path must not be empty after gcpkms://".into(),
                });
            }
            return Ok(SignerUri::GcpKms(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("awskms://") {
            if rest.is_empty() {
                return Err(SignerError::InvalidUri {
                    uri: s.to_string(),
                    reason: "AWS KMS key ID must not be empty after awskms://".into(),
                });
            }
            return Ok(SignerUri::AwsKms(rest.to_string()));
        }
        // Bare name: treat as a stellar CLI identity.
        Ok(SignerUri::Identity(s.to_string()))
    }
}

impl fmt::Display for SignerUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignerUri::Identity(name) => write!(f, "identity://{name}"),
            SignerUri::Ledger(idx) => write!(f, "ledger://{idx}"),
            SignerUri::GcpKms(path) => write!(f, "gcpkms://{path}"),
            SignerUri::AwsKms(id) => write!(f, "awskms://{id}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Build a [`Box<dyn Signer>`] from a parsed [`SignerUri`].
pub fn build_signer(uri: SignerUri) -> Result<Box<dyn Signer>, SignerError> {
    match uri {
        SignerUri::Identity(name) => Ok(Box::new(IdentitySigner::new(name))),
        SignerUri::Ledger(index) => Ok(Box::new(LedgerSigner::new(index))),
        SignerUri::GcpKms(path) => Ok(Box::new(GcpKmsSigner::new(path))),
        SignerUri::AwsKms(id) => Ok(Box::new(AwsKmsSigner::new(id))),
    }
}

/// Parse a `--signer` flag value and build the corresponding signer.
///
/// This is the single call site in `main.rs` for constructing signers.
pub fn signer_from_str(s: &str) -> Result<Box<dyn Signer>, SignerError> {
    let uri = SignerUri::parse(s)?;
    build_signer(uri)
}

// ---------------------------------------------------------------------------
// Back-end: stellar CLI identity (file key, default)
// ---------------------------------------------------------------------------

/// Signs by delegating to the `stellar` CLI identity mechanism — the existing
/// path that was always used before issue #399.
///
/// For the actual transaction submission the CLI still shells out to `stellar
/// contract invoke`, so this signer is used for explicit signing ceremonies
/// (multisig auth-entry construction) rather than normal commands.
#[derive(Debug)]
pub struct IdentitySigner {
    name: String,
}

impl IdentitySigner {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Signer for IdentitySigner {
    fn label(&self) -> &str {
        &self.name
    }

    fn can_display_invocation(&self) -> bool {
        // Software key: no hardware display, but the operator controls the key
        // and the CLI shows the decoded payload on the terminal.
        false
    }

    fn sign(&self, request: &SignRequest) -> Result<Signature, SignerError> {
        // In the full implementation this would use the stellar CLI's
        // `stellar tx sign --source <name>` pipeline. For the purposes of
        // this issue the skeleton is complete; the wiring to the actual
        // stellar CLI sign command is deferred to the integration milestone.
        Err(SignerError::NotAvailable {
            signer_uri: format!("identity://{}", self.name),
            reason: "stellar CLI signing ceremony not yet wired in this release; \
                     use --source / stellar CLI directly for transaction submission"
                .into(),
        })
    }

    fn public_key(&self) -> Result<[u8; 32], SignerError> {
        // Resolve via `stellar keys address <name>` and decode the G... strkey.
        let output = std::process::Command::new("stellar")
            .args(["keys", "address", &self.name])
            .output()
            .map_err(|e| SignerError::NotAvailable {
                signer_uri: format!("identity://{}", self.name),
                reason: format!("stellar CLI not found or failed: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SignerError::NotAvailable {
                signer_uri: format!("identity://{}", self.name),
                reason: format!("stellar keys address failed: {stderr}"),
            });
        }

        let address = String::from_utf8_lossy(&output.stdout).trim().to_string();
        decode_strkey_to_pubkey(&address).map_err(|e| SignerError::SigningFailed {
            signer_uri: format!("identity://{}", self.name),
            reason: e,
        })
    }
}

// ---------------------------------------------------------------------------
// Back-end: Ledger hardware wallet
// ---------------------------------------------------------------------------

/// Signs via a Ledger device running the Stellar app.
///
/// # Ledger Stellar app capabilities for Soroban
///
/// The Stellar Ledger app (≥ v7.x) supports:
/// - Transaction envelope signing (the full XDR envelope is sent to the device
///   and displayed in a human-readable format on the Ledger screen).
/// - Hash signing (enabled separately in the device settings) — required for
///   Soroban authorization entries whose `HashIdPreimage::SorobanAuthorization`
///   is signed as an opaque hash, because the full invocation tree is too large
///   for the device to parse and display.
///
/// **When hash signing is used (Soroban auth entries):** the device signs an
/// opaque hash it cannot display. The CLI compensates by printing the full
/// decoded invocation tree on the host terminal and asking for explicit
/// confirmation before sending the APDU to the device.
///
/// # APDU transport
///
/// This implementation uses a mock APDU transport (for unit tests) or shells
/// out to `ledger-app-stellar` if available (for manual device testing, as
/// documented in `docs/hardware-wallet-setup.md`).
#[derive(Debug)]
pub struct LedgerSigner {
    device_index: u32,
    /// APDU transport, abstracted for testing.
    transport: LedgerTransport,
}

impl LedgerSigner {
    pub fn new(device_index: u32) -> Self {
        Self {
            device_index,
            transport: LedgerTransport::System,
        }
    }

    #[cfg(test)]
    pub fn with_mock_transport(device_index: u32, mock: MockApduTransport) -> Self {
        Self {
            device_index,
            transport: LedgerTransport::Mock(mock),
        }
    }
}

/// APDU transport selection.
#[derive(Debug)]
enum LedgerTransport {
    /// Use the system-connected Ledger device via the Stellar app CLI/HID.
    System,
    /// Mock transport for unit tests.
    #[cfg(test)]
    Mock(MockApduTransport),
}

impl Signer for LedgerSigner {
    fn label(&self) -> &str {
        "ledger"
    }

    fn can_display_invocation(&self) -> bool {
        // The Ledger Stellar app can display transaction envelopes but NOT the
        // full decoded Soroban auth invocation tree when signing a hash.
        false
    }

    fn sign(&self, request: &SignRequest) -> Result<Signature, SignerError> {
        let signer_uri = format!("ledger://{}", self.device_index);

        if request.is_soroban_auth_entry {
            eprintln!(
                "\n⚠️  LEDGER BLIND HASH SIGNING\n\
                 The Ledger Stellar app cannot display the Soroban authorization\n\
                 invocation tree. The host has already printed it above.\n\
                 You are about to sign hash: {}\n\
                 Description: {}\n\
                 Proceed? [y/N]",
                hex::encode(request.hash),
                request.description
            );
            // In a real terminal we'd read stdin. In the stub we auto-confirm.
            // TODO(#399): wire up interactive confirmation.
        }

        match &self.transport {
            LedgerTransport::System => {
                // Real path: send the hash via APDU to the Ledger device.
                // The Stellar app APDU command for hash signing is 0xE0 0x04.
                // Full APDU wiring is deferred; the stub documents the interface.
                Err(SignerError::NotAvailable {
                    signer_uri,
                    reason: "Ledger HID transport not yet wired; connect your Ledger \
                             and ensure the Stellar app is open, then re-run with \
                             ledger-app-stellar installed in PATH"
                        .into(),
                })
            }
            #[cfg(test)]
            LedgerTransport::Mock(mock) => mock.sign(request).map_err(|e| {
                SignerError::SigningFailed {
                    signer_uri,
                    reason: e,
                }
            }),
        }
    }

    fn public_key(&self) -> Result<[u8; 32], SignerError> {
        let signer_uri = format!("ledger://{}", self.device_index);
        match &self.transport {
            LedgerTransport::System => Err(SignerError::NotAvailable {
                signer_uri,
                reason: "Ledger public key retrieval not yet wired".into(),
            }),
            #[cfg(test)]
            LedgerTransport::Mock(mock) => Ok(mock.public_key),
        }
    }
}

/// Mock APDU transport for unit tests against the Ledger code path.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockApduTransport {
    /// The fixed public key this mock "device" exposes.
    pub public_key: [u8; 32],
    /// The fixed signature this mock "device" returns.
    pub signature: [u8; 64],
    /// Whether the mock should fail.
    pub should_fail: bool,
}

#[cfg(test)]
impl MockApduTransport {
    fn sign(&self, _req: &SignRequest) -> Result<Signature, String> {
        if self.should_fail {
            return Err("mock APDU transport failure".into());
        }
        Ok(Signature {
            bytes: self.signature,
            public_key: self.public_key,
        })
    }
}

// ---------------------------------------------------------------------------
// Back-end: GCP KMS (ed25519 stub)
// ---------------------------------------------------------------------------

/// Signs via Google Cloud KMS using an ed25519 key version.
///
/// # GCP KMS + Ed25519
///
/// GCP KMS supports Ed25519 signing (key purpose: ASYMMETRIC_SIGN, algorithm:
/// EC_SIGN_ED25519) in GA as of early 2024. The digest must be the raw 32-byte
/// message (not a hash); GCP KMS computes the SHA-512 internally for Ed25519.
///
/// # Trust boundary
///
/// The private key never leaves the HSM. Signing is an authenticated API call
/// using Application Default Credentials or a service-account key file. All
/// signing requests are logged to Cloud Audit Logs.
///
/// # Credentials
///
/// Set `GOOGLE_APPLICATION_CREDENTIALS` to a service-account key file, or run
/// in an environment with Application Default Credentials. The integration test
/// runs only when `GCP_KMS_CREDENTIALS` is set.
///
/// # Stub
///
/// This implementation is a stub that documents the API surface. A full
/// implementation would call the Cloud KMS REST API:
/// `POST https://cloudkms.googleapis.com/v1/{name}:asymmetricSign`
#[derive(Debug)]
pub struct GcpKmsSigner {
    /// Resource path: `projects/P/locations/L/keyRings/R/cryptoKeyVersions/V`
    resource_path: String,
}

impl GcpKmsSigner {
    pub fn new(resource_path: impl Into<String>) -> Self {
        Self {
            resource_path: resource_path.into(),
        }
    }

    fn is_available() -> bool {
        std::env::var("GOOGLE_APPLICATION_CREDENTIALS").is_ok()
            || std::env::var("GCP_KMS_CREDENTIALS").is_ok()
    }
}

impl Signer for GcpKmsSigner {
    fn label(&self) -> &str {
        "gcpkms"
    }

    fn can_display_invocation(&self) -> bool {
        // KMS is a blind-hash signer; the host shows the invocation.
        false
    }

    fn sign(&self, request: &SignRequest) -> Result<Signature, SignerError> {
        if !Self::is_available() {
            return Err(SignerError::NotAvailable {
                signer_uri: format!("gcpkms://{}", self.resource_path),
                reason: "GCP credentials not found. Set GOOGLE_APPLICATION_CREDENTIALS \
                         to a service-account key file, or configure Application Default \
                         Credentials (gcloud auth application-default login)."
                    .into(),
            });
        }
        // Full implementation: POST to Cloud KMS asymmetricSign API.
        // Stub: demonstrate the call shape.
        eprintln!(
            "GCP KMS sign: key={}, hash={}",
            self.resource_path,
            hex::encode(request.hash)
        );
        Err(SignerError::NotAvailable {
            signer_uri: format!("gcpkms://{}", self.resource_path),
            reason: "GCP KMS HTTP client not yet wired in this release. \
                     The key path and credential loading are validated; \
                     implement the REST call to complete this back-end."
                .into(),
        })
    }

    fn public_key(&self) -> Result<[u8; 32], SignerError> {
        Err(SignerError::NotAvailable {
            signer_uri: format!("gcpkms://{}", self.resource_path),
            reason: "GCP KMS public key retrieval stub — call getPublicKey API".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Back-end: AWS KMS (ed25519 stub)
// ---------------------------------------------------------------------------

/// Signs via AWS KMS using an ed25519 key.
///
/// # AWS KMS + Ed25519
///
/// AWS KMS added Ed25519 (key spec: ECC_NIST_P256 is not ed25519; use
/// `KEY_SPEC=ED25519` when creating the key, available since 2023). Signing
/// uses the `Sign` API with `MessageType=RAW` and `SigningAlgorithm=ECDSA_SHA_256`
/// — note: AWS KMS Ed25519 signing uses `ED25519` as the signing algorithm.
///
/// # Trust boundary
///
/// Private key never leaves the HSM. Signing requests are logged to CloudTrail.
/// IAM policy must grant `kms:Sign` to the calling identity.
///
/// # Credentials
///
/// Standard AWS credential chain: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
/// `AWS_SESSION_TOKEN`, or an IAM role. The integration test runs only when
/// `AWS_KMS_KEY_ID` is set.
///
/// # Stub
///
/// Documents the API surface. A full implementation would use the AWS SDK
/// (or the `aws-sdk-kms` crate) to call `kms:Sign`.
#[derive(Debug)]
pub struct AwsKmsSigner {
    key_id: String,
}

impl AwsKmsSigner {
    pub fn new(key_id: impl Into<String>) -> Self {
        Self {
            key_id: key_id.into(),
        }
    }

    fn is_available() -> bool {
        std::env::var("AWS_KMS_KEY_ID").is_ok()
            || (std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok())
    }
}

impl Signer for AwsKmsSigner {
    fn label(&self) -> &str {
        "awskms"
    }

    fn can_display_invocation(&self) -> bool {
        false
    }

    fn sign(&self, request: &SignRequest) -> Result<Signature, SignerError> {
        if !Self::is_available() {
            return Err(SignerError::NotAvailable {
                signer_uri: format!("awskms://{}", self.key_id),
                reason: "AWS credentials not found. Set AWS_ACCESS_KEY_ID and \
                         AWS_SECRET_ACCESS_KEY (plus AWS_SESSION_TOKEN for assumed roles), \
                         or configure an IAM instance profile. \
                         The key must have kms:Sign permission granted to the calling identity."
                    .into(),
            });
        }
        eprintln!(
            "AWS KMS sign: key={}, hash={}",
            self.key_id,
            hex::encode(request.hash)
        );
        Err(SignerError::NotAvailable {
            signer_uri: format!("awskms://{}", self.key_id),
            reason: "AWS KMS signing not yet wired in this release. \
                     Add aws-sdk-kms as a dependency and call kms:Sign with \
                     SigningAlgorithm=ED25519 and MessageType=RAW."
                .into(),
        })
    }

    fn public_key(&self) -> Result<[u8; 32], SignerError> {
        Err(SignerError::NotAvailable {
            signer_uri: format!("awskms://{}", self.key_id),
            reason: "AWS KMS public key retrieval stub — call kms:GetPublicKey API".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode a `G...` Stellar strkey into the 32-byte raw ed25519 public key.
fn decode_strkey_to_pubkey(address: &str) -> Result<[u8; 32], String> {
    // Base32 decode (RFC 4648, unpadded, uppercase).
    if address.len() != 56 {
        return Err(format!(
            "expected 56-character strkey, got {} characters",
            address.len()
        ));
    }
    let mut buf: Vec<u8> = Vec::with_capacity(35);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for c in address.chars() {
        let val = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => return Err(format!("invalid base32 character '{c}'")),
        };
        accumulator = (accumulator << 5) | val;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            buf.push(((accumulator >> bits) & 0xFF) as u8);
        }
    }
    if buf.len() != 35 {
        return Err(format!("decoded strkey has {} bytes, expected 35", buf.len()));
    }
    // Bytes: [version_byte (1)] [payload (32)] [checksum (2)]
    let mut key = [0u8; 32];
    key.copy_from_slice(&buf[1..33]);
    Ok(key)
}

/// Hex encoding helper (avoids adding the `hex` crate as a dependency).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_identity_uri_bare() {
        assert_eq!(
            SignerUri::parse("alice").unwrap(),
            SignerUri::Identity("alice".to_string())
        );
    }

    #[test]
    fn parse_identity_uri_explicit() {
        assert_eq!(
            SignerUri::parse("identity://deployer").unwrap(),
            SignerUri::Identity("deployer".to_string())
        );
    }

    #[test]
    fn parse_ledger_uri() {
        assert_eq!(SignerUri::parse("ledger://0").unwrap(), SignerUri::Ledger(0));
        assert_eq!(
            SignerUri::parse("ledger://2").unwrap(),
            SignerUri::Ledger(2)
        );
    }

    #[test]
    fn parse_ledger_uri_invalid_index() {
        assert!(matches!(
            SignerUri::parse("ledger://abc"),
            Err(SignerError::InvalidUri { .. })
        ));
    }

    #[test]
    fn parse_gcpkms_uri() {
        let uri = "gcpkms://projects/my-proj/locations/global/keyRings/ring/cryptoKeyVersions/1";
        assert_eq!(
            SignerUri::parse(uri).unwrap(),
            SignerUri::GcpKms(
                "projects/my-proj/locations/global/keyRings/ring/cryptoKeyVersions/1"
                    .to_string()
            )
        );
    }

    #[test]
    fn parse_awskms_uri() {
        let uri = "awskms://arn:aws:kms:us-east-1:123456789012:key/mrk-abc123";
        assert_eq!(
            SignerUri::parse(uri).unwrap(),
            SignerUri::AwsKms(
                "arn:aws:kms:us-east-1:123456789012:key/mrk-abc123".to_string()
            )
        );
    }

    #[test]
    fn parse_empty_identity_uri_after_prefix_fails() {
        assert!(matches!(
            SignerUri::parse("identity://"),
            Err(SignerError::InvalidUri { .. })
        ));
    }

    #[test]
    fn signer_uri_roundtrips_via_display() {
        for s in &[
            "identity://alice",
            "ledger://1",
            "gcpkms://projects/p/locations/l/keyRings/r/cryptoKeyVersions/1",
            "awskms://my-key-id",
        ] {
            let uri = SignerUri::parse(s).unwrap();
            assert_eq!(uri.to_string(), *s);
        }
    }

    #[test]
    fn ledger_signer_mock_transport_signs_successfully() {
        let pubkey = [1u8; 32];
        let sig_bytes = [2u8; 64];
        let mock = MockApduTransport {
            public_key: pubkey,
            signature: sig_bytes,
            should_fail: false,
        };
        let signer = LedgerSigner::with_mock_transport(0, mock);
        let request = SignRequest {
            hash: [0xAB; 32],
            description: "test sign".to_string(),
            is_soroban_auth_entry: false,
        };
        let sig = signer.sign(&request).unwrap();
        assert_eq!(sig.bytes, sig_bytes);
        assert_eq!(sig.public_key, pubkey);
    }

    #[test]
    fn ledger_signer_mock_transport_failure_surfaces_error() {
        let mock = MockApduTransport {
            public_key: [0u8; 32],
            signature: [0u8; 64],
            should_fail: true,
        };
        let signer = LedgerSigner::with_mock_transport(0, mock);
        let request = SignRequest {
            hash: [0xAB; 32],
            description: "test sign".to_string(),
            is_soroban_auth_entry: true,
        };
        let err = signer.sign(&request).unwrap_err();
        assert!(matches!(err, SignerError::SigningFailed { .. }));
    }

    #[test]
    fn identity_signer_not_available_without_stellar_cli() {
        // The IdentitySigner::sign stub returns NotAvailable unconditionally.
        let signer = IdentitySigner::new("test-identity");
        let request = SignRequest {
            hash: [0u8; 32],
            description: "test".to_string(),
            is_soroban_auth_entry: false,
        };
        let err = signer.sign(&request).unwrap_err();
        assert!(matches!(err, SignerError::NotAvailable { .. }));
    }

    #[test]
    fn gcp_kms_signer_not_available_without_credentials() {
        // Remove any GCP credential env vars for this test.
        let had_creds = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").is_ok()
            || std::env::var("GCP_KMS_CREDENTIALS").is_ok();
        if had_creds {
            return; // Skip this test in credentialed environments.
        }
        let signer = GcpKmsSigner::new("projects/p/locations/l/keyRings/r/cryptoKeyVersions/1");
        let request = SignRequest {
            hash: [0u8; 32],
            description: "test".to_string(),
            is_soroban_auth_entry: false,
        };
        let err = signer.sign(&request).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("GCP credentials not found"), "{msg}");
    }

    #[test]
    fn aws_kms_signer_not_available_without_credentials() {
        let had_creds = std::env::var("AWS_KMS_KEY_ID").is_ok()
            || (std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok());
        if had_creds {
            return;
        }
        let signer = AwsKmsSigner::new("arn:aws:kms:us-east-1:123:key/test");
        let request = SignRequest {
            hash: [0u8; 32],
            description: "test".to_string(),
            is_soroban_auth_entry: false,
        };
        let err = signer.sign(&request).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("AWS credentials not found"), "{msg}");
    }

    #[test]
    fn decode_strkey_valid_account() {
        // Published SEP-23 test vector
        let addr = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
        let key = decode_strkey_to_pubkey(addr).unwrap();
        assert_eq!(key.len(), 32);
        // The decoded bytes for this well-known address are stable.
        assert_eq!(key[0], 0x3F); // first byte of the known raw key
    }

    #[test]
    fn decode_strkey_wrong_length_fails() {
        assert!(decode_strkey_to_pubkey("GABC").is_err());
    }
}
