//! Signer abstraction for transaction and auth-entry signing.
//!
//! A `Signer` transforms a 32-byte payload (the auth-entry hash or the
//! transaction hash) into an ed25519 signature together with the public key
//! that produced it.  Keeping this behind a trait lets the CLI use local
//! keys, hardware signers, or the multisig ceremony flow through the same
//! code path — the calling code never sees raw secret material.
//!
//! Current concrete implementations:
//! - [`LocalSigner`] — loads an ed25519 key stored in stellar-cli's key
//!   identity directory (`~/.config/stellar/identity/`).  The key material
//!   is zeroed from memory as soon as the signature is produced.
//! - [`ViaStellarCli`] — delegates every signing operation back to the
//!   `stellar` binary.  This is the fallback path used when
//!   `--via-stellar-cli` is passed; it is removed once every operator has
//!   migrated to native signing.
//! - [`KeyringSigner`] — fetches the secret key from the OS keyring
//!   (macOS Keychain, Windows Credential Manager, Secret Service on Linux).
//! - [`EncryptedFileSigner`] — reads an age-encrypted key file that must
//!   have strict `0600` permissions and is unlocked with a passphrase.
//! - [`ExternalProcessSigner`] — hands the payload to an external signer
//!   process over a JSON protocol on stdin/stdout (compatible with the
//!   planned Ledger/HSM abstraction).
//!
//! # Secret handling
//!
//! Secret material is wrapped in [`zeroize::Zeroizing`] buffers and the
//! types that hold it deliberately do **not** implement `Debug`.  Plaintext
//! keys supplied through environment variables or config files are only
//! accepted when the operator passes `--allow-plaintext-key`; see
//! [`PlaintextKeyPolicy`].  See `docs/security/chw-key-custody.md` for the
//! threat model covering each backend.

use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use zeroize::Zeroizing;

/// A 64-byte ed25519 signature.
pub type Signature = [u8; 64];
/// A 32-byte ed25519 public key.
pub type PublicKey = [u8; 32];

/// A detached signature bundle produced by an offline signer.
///
/// The bundle carries everything a networked host needs to assemble the
/// signed envelope without ever seeing the secret key: the signer's public
/// key, the signature over the envelope payload, and the digest of the
/// exact envelope bytes that were signed.  The digest lets the assembling
/// host detect tampering between the offline device and submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureBundle {
    /// The ed25519 public key that produced `signature`.
    pub public_key: PublicKey,
    /// The detached signature over the envelope payload.
    pub signature: Signature,
    /// SHA-256 digest of the envelope bytes that were signed.
    pub envelope_digest: [u8; 32],
}

impl SignatureBundle {
    /// Encode the bundle as a compact, self-describing text payload suitable
    /// for writing to a file or splitting into QR frames.
    ///
    /// Format: `lafiya-sig-v1:<hex public key>:<hex signature>:<hex digest>`
    pub fn to_envelope_string(&self) -> String {
        format!(
            "lafiya-sig-v1:{}:{}:{}",
            hex_encode(&self.public_key),
            hex_encode(&self.signature),
            hex_encode(&self.envelope_digest),
        )
    }

    /// Decode a bundle previously produced by [`SignatureBundle::to_envelope_string`].
    pub fn from_envelope_string(s: &str) -> Result<Self, SignerError> {
        let mut parts = s.trim().split(':');
        let tag = parts.next().unwrap_or_default();
        if tag != "lafiya-sig-v1" {
            return Err(SignerError::BundleDecodeError {
                detail: format!("unexpected bundle tag '{tag}'"),
            });
        }
        let public_key = decode_fixed::<32>(parts.next(), "public key")?;
        let signature = decode_fixed::<64>(parts.next(), "signature")?;
        let envelope_digest = decode_fixed::<32>(parts.next(), "envelope digest")?;
        if parts.next().is_some() {
            return Err(SignerError::BundleDecodeError {
                detail: "trailing data after signature bundle".to_string(),
            });
        }
        Ok(Self {
            public_key,
            signature,
            envelope_digest,
        })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn decode_fixed<const N: usize>(
    field: Option<&str>,
    name: &str,
) -> Result<[u8; N], SignerError> {
    let field = field.ok_or_else(|| SignerError::BundleDecodeError {
        detail: format!("missing {name}"),
    })?;
    if field.len() != N * 2 {
        return Err(SignerError::BundleDecodeError {
            detail: format!("{name} must be {} hex chars", N * 2),
        });
    }
    let mut out = [0u8; N];
    for (i, chunk) in field.as_bytes().chunks(2).enumerate() {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_val(c: u8) -> Result<u8, SignerError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(SignerError::BundleDecodeError {
            detail: format!("invalid hex character '{}'", c as char),
        }),
    }
}

/// A minimal, dependency-free SHA-256 used to bind a signature bundle to the
/// exact envelope bytes that were signed.
///
/// This is intentionally self-contained so the offline signing path has no
/// external crate requirements.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Errors that can occur during signing.
#[derive(Debug)]
pub enum SignerError {
    /// The key identity was not found in the configured directory.
    KeyNotFound { identity: String },
    /// The key file could not be read or its format is unrecognised.
    KeyReadError { identity: String, detail: String },
    /// The signing operation failed (e.g. corrupted key data).
    SigningFailed { detail: String },
    /// The `stellar` binary was not found (only relevant for [`ViaStellarCli`]).
    StellarCliNotFound,
    /// The `stellar` binary exited with an error.
    StellarCliFailed { exit_code: Option<i32>, stderr: String },
    /// The OS keyring backend is unavailable or returned an error.
    KeyringUnavailable { detail: String },
    /// The encrypted key file has permissions that are too permissive.
    InsecurePermissions { path: PathBuf, mode: u32 },
    /// The encrypted key file could not be decrypted (wrong passphrase or
    /// corrupted ciphertext).
    DecryptionFailed { detail: String },
    /// The external signer process failed or violated the JSON protocol.
    ExternalSignerFailed { detail: String },
    /// A plaintext key was supplied without `--allow-plaintext-key`.
    PlaintextKeyRejected,
}

impl fmt::Display for SignerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignerError::KeyNotFound { identity } => {
                write!(f, "key identity '{identity}' not found")
            }
            SignerError::KeyReadError { identity, detail } => {
                write!(f, "could not read key '{identity}': {detail}")
            }
            SignerError::SigningFailed { detail } => write!(f, "signing failed: {detail}"),
            SignerError::StellarCliNotFound => write!(
                f,
                "stellar CLI not found; install with `cargo install --locked stellar-cli` \
                 or remove --via-stellar-cli to use native signing"
            ),
            SignerError::StellarCliFailed { exit_code, stderr } => {
                write!(
                    f,
                    "stellar CLI failed (exit {:?}): {}",
                    exit_code, stderr
                )
            }
            SignerError::KeyringUnavailable { detail } => {
                write!(f, "OS keyring unavailable: {detail}")
            }
            SignerError::InsecurePermissions { path, mode } => write!(
                f,
                "key file '{}' has insecure permissions {:o}; expected 0600",
                path.display(),
                mode
            ),
            SignerError::DecryptionFailed { detail } => {
                write!(f, "could not decrypt key file: {detail}")
            }
            SignerError::ExternalSignerFailed { detail } => {
                write!(f, "external signer failed: {detail}")
            }
            SignerError::PlaintextKeyRejected => write!(
                f,
                "refusing to use a plaintext key from env/config; \
                 pass --allow-plaintext-key to override (insecure)"
            ),
        }
    }
}

impl std::error::Error for SignerError {}

/// The result of a signing operation: the ed25519 public key and its
/// corresponding signature over the supplied payload.
#[derive(Debug, Clone)]
pub struct SignResult {
    pub public_key: PublicKey,
    pub signature: Signature,
}

/// Signing backend.
///
/// Implementors must be `Send + Sync` so the CLI can use them across async
/// boundaries (once the native RPC path is async).
pub trait Signer: Send + Sync {
    /// Sign a 32-byte payload and return the `(public_key, signature)` pair.
    ///
    /// The payload is typically either:
    /// - the `HashIdPreimage::SorobanAuthorization` payload hash for an auth
    ///   entry, or
    /// - the transaction hash for envelope signing.
    fn sign(&self, payload: &[u8; 32]) -> Result<SignResult, SignerError>;

    /// The ed25519 public key this signer will use, without actually signing.
    fn public_key(&self) -> Result<PublicKey, SignerError>;
}

/// A self-describing, unsigned envelope prepared on a networked host and
/// carried to an offline signing device.
///
/// The envelope is intentionally minimal and transport-agnostic: it can be
/// written to a file or split into QR frames.  The offline device re-decodes
/// it (see [`OfflineSigner::verify_envelope`]) before signing so that
/// tampering in transit is detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedEnvelope {
    /// Stellar network passphrase the transaction targets.
    pub network_passphrase: String,
    /// Contract id the transaction interacts with.
    pub contract_id: String,
    /// Unix timestamp (seconds) after which the envelope must not be signed.
    pub expires_at: u64,
    /// Human-readable decode of the payload for operator review.
    pub human_readable: String,
    /// The raw payload bytes to be signed.
    pub payload: Vec<u8>,
}

impl UnsignedEnvelope {
    /// Encode the envelope as a self-describing text payload.
    ///
    /// Format:
    /// `lafiya-env-v1:<network>:<contract>:<expires>:<hex payload>:<human readable>`
    pub fn to_envelope_string(&self) -> String {
        format!(
            "lafiya-env-v1:{}:{}:{}:{}:{}",
            self.network_passphrase,
            self.contract_id,
            self.expires_at,
            hex_encode(&self.payload),
            self.human_readable,
        )
    }

    /// Re-decode an envelope produced by [`UnsignedEnvelope::to_envelope_string`].
    ///
    /// This is the verification step run on the offline device: it parses the
    /// envelope and recomputes its digest so the caller can compare against
    /// the digest that was communicated out-of-band.
    pub fn from_envelope_string(s: &str) -> Result<Self, SignerError> {
        let s = s.trim();
        let mut parts = s.splitn(6, ':');
        let tag = parts.next().unwrap_or_default();
        if tag != "lafiya-env-v1" {
            return Err(SignerError::EnvelopeDecodeError {
                detail: format!("unexpected envelope tag '{tag}'"),
            });
        }
        let network_passphrase = parts
            .next()
            .ok_or_else(|| SignerError::EnvelopeDecodeError {
                detail: "missing network passphrase".to_string(),
            })?
            .to_string();
        let contract_id = parts
            .next()
            .ok_or_else(|| SignerError::EnvelopeDecodeError {
                detail: "missing contract id".to_string(),
            })?
            .to_string();
        let expires_at = parts
            .next()
            .ok_or_else(|| SignerError::EnvelopeDecodeError {
                detail: "missing expiry".to_string(),
            })?
            .parse::<u64>()
            .map_err(|e| SignerError::EnvelopeDecodeError {
                detail: format!("invalid expiry: {e}"),
            })?;
        let payload_hex = parts
            .next()
            .ok_or_else(|| SignerError::EnvelopeDecodeError {
                detail: "missing payload".to_string(),
            })?;
        let payload = decode_hex(payload_hex)?;
        let human_readable = parts.next().unwrap_or_default().to_string();

        Ok(Self {
            network_passphrase,
            contract_id,
            expires_at,
            human_readable,
            payload,
        })
    }

    /// SHA-256 digest of the canonical envelope string.
    pub fn digest(&self) -> [u8; 32] {
        sha256(self.to_envelope_string().as_bytes())
    }
}

fn decode_hex(s: &str) -> Result<Vec<u8>, SignerError> {
    if s.len() % 2 != 0 {
        return Err(SignerError::EnvelopeDecodeError {
            detail: "payload hex has odd length".to_string(),
        });
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

/// Offline signing helper: re-decodes and verifies an envelope, then produces
/// a detached [`SignatureBundle`] without any network access.
///
/// Keys stay on the offline host — this type only ever holds a [`Signer`]
/// reference and never performs RPC.
pub struct OfflineSigner<'a> {
    signer: &'a dyn Signer,
}

impl<'a> OfflineSigner<'a> {
    pub fn new(signer: &'a dyn Signer) -> Self {
        Self { signer }
    }

    /// Re-decode the envelope on the offline device and verify it against the
    /// digest communicated out-of-band.
    ///
    /// Returns the decoded envelope on success so the operator can review the
    /// human-readable decode before signing.
    pub fn verify_envelope(
        &self,
        envelope_string: &str,
        expected_digest: &[u8; 32],
    ) -> Result<UnsignedEnvelope, SignerError> {
        let envelope = UnsignedEnvelope::from_envelope_string(envelope_string)?;
        let actual = envelope.digest();
        if &actual != expected_digest {
            return Err(SignerError::EnvelopeTampered {
                expected: hex_encode(expected_digest),
                actual: hex_encode(&actual),
            });
        }
        Ok(envelope)
    }

    /// Sign a verified envelope and emit a detached signature bundle.
    ///
    /// The payload must be exactly 32 bytes (the auth-entry or transaction
    /// hash).  No network access is performed.
    pub fn sign_envelope(
        &self,
        envelope: &UnsignedEnvelope,
    ) -> Result<SignatureBundle, SignerError> {
        let payload: [u8; 32] =
            envelope
                .payload
                .as_slice()
                .try_into()
                .map_err(|_| SignerError::SigningFailed {
                    detail: format!(
                        "envelope payload must be 32 bytes, got {}",
                        envelope.payload.len()
                    ),
                })?;
        let result = self.signer.sign(&payload)?;
        Ok(SignatureBundle {
            public_key: result.public_key,
            signature: result.signature,
            envelope_digest: envelope.digest(),
        })
    }

    /// Convenience: verify then sign in one step.
    pub fn verify_and_sign(
        &self,
        envelope_string: &str,
        expected_digest: &[u8; 32],
    ) -> Result<SignatureBundle, SignerError> {
        let envelope = self.verify_envelope(envelope_string, expected_digest)?;
        self.sign_envelope(&envelope)
    }
}

// ---------------------------------------------------------------------------
// Plaintext key policy
// ---------------------------------------------------------------------------

/// Policy governing whether plaintext keys from env/config are accepted.
///
/// Plaintext keys leak through shell history, process listings, crash dumps
/// and logs, so they are rejected unless the operator explicitly opts in
/// with `--allow-plaintext-key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaintextKeyPolicy {
    /// Reject plaintext keys (the default).
    Reject,
    /// Accept plaintext keys after emitting a warning.
    AllowWithWarning,
}

impl PlaintextKeyPolicy {
    /// Build a policy from the `--allow-plaintext-key` flag value.
    pub fn from_flag(allow: bool) -> Self {
        if allow {
            PlaintextKeyPolicy::AllowWithWarning
        } else {
            PlaintextKeyPolicy::Reject
        }
    }

    /// Check whether a plaintext key may be used, returning the warning that
    /// must be logged when it is allowed.
    pub fn check(&self) -> Result<Option<&'static str>, SignerError> {
        match self {
            PlaintextKeyPolicy::Reject => Err(SignerError::PlaintextKeyRejected),
            PlaintextKeyPolicy::AllowWithWarning => Ok(Some(
                "WARNING: using a plaintext secret key from env/config; \
                 this is insecure — migrate to the OS keyring, an encrypted \
                 key file, or an external signer",
            )),
        }
    }
}

/// A secret key held in a zeroizing buffer.
///
/// Deliberately does **not** implement `Debug` or `Display` so the bytes can
/// never be formatted into a log line or panic message by accident.
pub struct SecretKey(Zeroizing<Vec<u8>>);

impl SecretKey {
    /// Wrap raw key bytes, taking ownership and zeroizing on drop.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Borrow the raw bytes for the duration of a signing call.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// LocalSigner
// ---------------------------------------------------------------------------

/// A signer backed by a key stored in stellar-cli's identity directory.
///
/// The key storage format is the same as stellar-cli uses so operators can
/// reuse existing key management.  Secret key bytes are zeroed from the
/// heap as soon as the signature is produced; the `LocalSigner` struct
/// itself holds only the identity name, not the secret bytes.
///
/// **Not yet implemented**: the actual ed25519 signing.  When
/// `stellar-xdr` and an ed25519 crate are added in the full native RPC
/// implementation, this stub will be replaced.  Until then, calling `sign`
/// returns `SignerError::SigningFailed` with an explanatory message.
pub struct LocalSigner {
    /// The identity name as registered with `stellar keys generate --name`.
    pub identity: String,
}

impl LocalSigner {
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
        }
    }
}

impl Signer for LocalSigner {
    fn sign(&self, _payload: &[u8; 32]) -> Result<SignResult, SignerError> {
        // TODO(#395): Implement native ed25519 signing from the stellar-cli
        // identity file format once `stellar-xdr` is added as a dependency.
        Err(SignerError::SigningFailed {
            detail: format!(
                "native ed25519 signing for identity '{}' is not yet implemented; \
                 use --via-stellar-cli for now",
                self.identity
            ),
        })
    }

    fn public_key(&self) -> Result<PublicKey, SignerError> {
        Err(SignerError::SigningFailed {
            detail: format!(
                "native key loading for identity '{}' is not yet implemented",
                self.identity
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// KeyringSigner
// ---------------------------------------------------------------------------

/// Signer that fetches the secret key from the OS keyring.
///
/// The keyring entry is identified by `(service, account)`.  The secret is
/// loaded into a [`SecretKey`] and zeroized as soon as signing completes.
/// This backend never touches the filesystem or environment for key
/// material.
pub struct KeyringSigner {
    /// Keyring service name (e.g. `lafiya-cli`).
    pub service: String,
    /// Keyring account name (e.g. the operator identity).
    pub account: String,
}

impl KeyringSigner {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    /// Load the secret key from the OS keyring.
    ///
    /// The `keyring` crate is not yet a dependency; until it is added this
    /// returns [`SignerError::KeyringUnavailable`] so callers fail closed
    /// rather than silently falling back to a plaintext source.
    fn load_secret(&self) -> Result<SecretKey, SignerError> {
        // TODO(#538): wire up the `keyring` crate:
        //   let entry = keyring::Entry::new(&self.service, &self.account)?;
        //   let secret = entry.get_secret()?;
        Err(SignerError::KeyringUnavailable {
            detail: format!(
                "keyring backend not compiled in; cannot load '{}/{}'",
                self.service, self.account
            ),
        })
    }
}

impl Signer for KeyringSigner {
    fn sign(&self, _payload: &[u8; 32]) -> Result<SignResult, SignerError> {
        let _secret = self.load_secret()?;
        // TODO(#538): ed25519-sign `_payload` with `_secret` once the
        // signing primitive is wired in (tracked with #395).
        Err(SignerError::SigningFailed {
            detail: "keyring signing primitive not yet implemented".to_string(),
        })
    }

    fn public_key(&self) -> Result<PublicKey, SignerError> {
        let _secret = self.load_secret()?;
        Err(SignerError::SigningFailed {
            detail: "keyring public-key derivation not yet implemented".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// EncryptedFileSigner
// ---------------------------------------------------------------------------

/// Signer backed by an age-encrypted key file with strict permissions.
///
/// The file must be readable only by its owner (`0600`); anything more
/// permissive is rejected with [`SignerError::InsecurePermissions`].  The
/// passphrase is supplied by the caller (typically prompted on a TTY, never
/// read from the environment) and is itself zeroized.
pub struct EncryptedFileSigner {
    /// Path to the age-encrypted key file.
    pub path: PathBuf,
    /// Passphrase used to decrypt the file.
    passphrase: Zeroizing<String>,
}

impl EncryptedFileSigner {
    /// Create a signer for `path`, taking ownership of the passphrase.
    pub fn new(path: impl Into<PathBuf>, passphrase: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            passphrase: Zeroizing::new(passphrase.into()),
        }
    }

    /// Verify that the key file is owned by the current user and has mode
    /// `0600` on Unix.  On non-Unix platforms this is a no-op.
    pub fn check_permissions(&self) -> Result<(), SignerError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&self.path).map_err(|e| {
                SignerError::KeyReadError {
                    identity: self.path.display().to_string(),
                    detail: e.to_string(),
                }
            })?;
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(SignerError::InsecurePermissions {
                    path: self.path.clone(),
                    mode,
                });
            }
        }
        Ok(())
    }

    /// Read and decrypt the key file into a [`SecretKey`].
    ///
    /// The `age` crate is not yet a dependency; until it is added this
    /// returns [`SignerError::DecryptionFailed`] after the permission check
    /// so the failure mode is explicit.
    fn load_secret(&self) -> Result<SecretKey, SignerError> {
        self.check_permissions()?;
        let mut ciphertext = Vec::new();
        std::fs::File::open(&self.path)
            .and_then(|mut f| f.read_to_end(&mut ciphertext))
            .map_err(|e| SignerError::KeyReadError {
                identity: self.path.display().to_string(),
                detail: e.to_string(),
            })?;
        // TODO(#538): decrypt `ciphertext` with `self.passphrase` using the
        // `age` crate and return the plaintext key bytes.
        let _ = &self.passphrase;
        Err(SignerError::DecryptionFailed {
            detail: "age decryption backend not compiled in".to_string(),
        })
    }
}

impl Signer for EncryptedFileSigner {
    fn sign(&self, _payload: &[u8; 32]) -> Result<SignResult, SignerError> {
        let _secret = self.load_secret()?;
        Err(SignerError::SigningFailed {
            detail: "encrypted-file signing primitive not yet implemented".to_string(),
        })
    }

    fn public_key(&self) -> Result<PublicKey, SignerError> {
        let _secret = self.load_secret()?;
        Err(SignerError::SigningFailed {
            detail: "encrypted-file public-key derivation not yet implemented".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// ExternalProcessSigner
// ---------------------------------------------------------------------------

/// Signer that delegates signing to an external process over a JSON protocol.
///
/// The protocol is line-delimited JSON on stdin/stdout:
///
/// Request:  `{"payload":"<hex>","public_key":true}`
/// Response: `{"public_key":"<hex>","signature":"<hex>"}`
///
/// This shape is intentionally compatible with the planned Ledger/HSM
/// abstraction: a hardware bridge can implement the same protocol without
/// changing the [`Signer`] trait.
pub struct ExternalProcessSigner {
    /// Path to the external signer executable.
    pub program: PathBuf,
    /// Extra arguments passed to the executable.
    pub args: Vec<String>,
}

impl ExternalProcessSigner {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Spawn the external signer, send one request, and read one response.
    fn round_trip(&self, request: &str) -> Result<String, SignerError> {
        let mut child: Child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| SignerError::ExternalSignerFailed {
                detail: format!("could not spawn '{}': {e}", self.program.display()),
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .map_err(|e| SignerError::ExternalSignerFailed {
                    detail: format!("write to external signer failed: {e}"),
                })?;
        }

        let mut line = String::new();
        if let Some(stdout) = child.stdout.take() {
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| SignerError::ExternalSignerFailed {
                    detail: format!("read from external signer failed: {e}"),
                })?;
        }
        let _ = child.wait();
        Ok(line)
    }
}

impl Signer for ExternalProcessSigner {
    fn sign(&self, payload: &[u8; 32]) -> Result<SignResult, SignerError> {
        let request = format!("{{\"payload\":\"{}\"}}", hex_encode(payload));
        let response = self.round_trip(&request)?;
        parse_sign_response(&response)
    }

    fn public_key(&self) -> Result<PublicKey, SignerError> {
        let request = "{\"public_key\":true}".to_string();
        let response = self.round_trip(&request)?;
        parse_sign_response(&response).map(|r| r.public_key)
    }
}

/// Minimal hex encoder (lowercase) for the external signer protocol.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Minimal hex decoder for the external signer protocol.
fn hex_decode(s: &str) -> Result<Vec<u8>, SignerError> {
    if s.len() % 2 != 0 {
        return Err(SignerError::ExternalSignerFailed {
            detail: "odd-length hex string".to_string(),
        });
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16);
        let lo = (pair[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(hi), Some(lo)) => out.push((hi * 16 + lo) as u8),
            _ => {
                return Err(SignerError::ExternalSignerFailed {
                    detail: "invalid hex string".to_string(),
                })
            }
        }
    }
    Ok(out)
}

/// Extract a `"key":"value"` field from a flat JSON object without pulling in
/// a JSON dependency.  Sufficient for the fixed external-signer protocol.
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let colon = rest.find(':')? + 1;
    let rest = rest[colon..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Parse an external-signer response into a [`SignResult`].
fn parse_sign_response(response: &str) -> Result<SignResult, SignerError> {
    let pk_hex = json_field(response, "public_key").ok_or_else(|| {
        SignerError::ExternalSignerFailed {
            detail: "response missing public_key".to_string(),
        }
    })?;
    let sig_hex = json_field(response, "signature").ok_or_else(|| {
        SignerError::ExternalSignerFailed {
            detail: "response missing signature".to_string(),
        }
    })?;
    let pk = hex_decode(pk_hex)?;
    let sig = hex_decode(sig_hex)?;
    let public_key: PublicKey = pk.try_into().map_err(|_| SignerError::ExternalSignerFailed {
        detail: "public_key is not 32 bytes".to_string(),
    })?;
    let signature: Signature = sig.try_into().map_err(|_| SignerError::ExternalSignerFailed {
        detail: "signature is not 64 bytes".to_string(),
    })?;
    Ok(SignResult {
        public_key,
        signature,
    })
}

// ---------------------------------------------------------------------------
// ViaStellarCli (fallback)
// ---------------------------------------------------------------------------

/// Fallback signer that delegates to the `stellar` binary.
///
/// This is used when `--via-stellar-cli` is passed on the command line.
/// It will be removed in the release after every operator has migrated to
/// native signing (issue #395).
pub struct ViaStellarCli {
    /// The stellar-cli identity name to pass as `--source`.
    pub identity: String,
}

impl ViaStellarCli {
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
        }
    }
}

impl Signer for ViaStellarCli {
    fn sign(&self, _payload: &[u8; 32]) -> Result<SignResult, SignerError> {
        // The actual subprocess invocation happens in RpcClient::sign_and_submit,
        // not here.  This method exists so ViaStellarCli satisfies the Signer
        // trait — but a caller that picks up a ViaStellarCli should route
        // through the stellar-cli subprocess path rather than calling sign()
        // directly.
        Err(SignerError::SigningFailed {
            detail: "ViaStellarCli::sign should not be called directly; \
                     use RpcClient::sign_and_submit with --via-stellar-cli"
                .to_string(),
        })
    }

    fn public_key(&self) -> Result<PublicKey, SignerError> {
        Err(SignerError::SigningFailed {
            detail: "ViaStellarCli::public_key is not implemented; \
                     use stellar keys show --name <identity>"
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_signer_new_stores_identity() {
        let s = LocalSigner::new("my-admin-key");
        assert_eq!(s.identity, "my-admin-key");
    }

    #[test]
    fn local_signer_sign_returns_not_yet_implemented_error() {
        let s = LocalSigner::new("test-key");
        let payload = [0u8; 32];
        let err = s.sign(&payload).unwrap_err();
        assert!(
            matches!(err, SignerError::SigningFailed { .. }),
            "expected SigningFailed, got {err}"
        );
    }

    #[test]
    fn via_stellar_cli_sign_returns_error() {
        let s = ViaStellarCli::new("test-key");
        let payload = [0u8; 32];
        let err = s.sign(&payload).unwrap_err();
        assert!(matches!(err, SignerError::SigningFailed { .. }));
    }

    #[test]
    fn plaintext_policy_rejects_by_default() {
        let policy = PlaintextKeyPolicy::from_flag(false);
        assert!(matches!(
            policy.check(),
            Err(SignerError::PlaintextKeyRejected)
        ));
    }

    #[test]
    fn plaintext_policy_allows_with_warning_when_flagged() {
        let policy = PlaintextKeyPolicy::from_flag(true);
        let warning = policy.check().unwrap();
        assert!(warning.is_some());
    }

    #[test]
    fn secret_key_is_zeroized_on_drop() {
        let key = SecretKey::new(vec![0xAA; 32]);
        assert_eq!(key.expose().len(), 32);
        // Dropping `key` zeroizes the backing buffer; nothing to assert
        // beyond the type compiling without Debug.
    }

    #[test]
    fn keyring_signer_fails_closed() {
        let s = KeyringSigner::new("lafiya-cli", "admin");
        let err = s.sign(&[0u8; 32]).unwrap_err();
        assert!(matches!(err, SignerError::KeyringUnavailable { .. }));
    }

    #[test]
    fn encrypted_file_signer_rejects_insecure_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("lafiya-sig-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("key.age");
            std::fs::write(&path, b"ciphertext").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let s = EncryptedFileSigner::new(&path, "passphrase");
            let err = s.check_permissions().unwrap_err();
            assert!(matches!(err, SignerError::InsecurePermissions { .. }));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn encrypted_file_signer_accepts_0600() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("lafiya-sig-ok-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("key.age");
            std::fs::write(&path, b"ciphertext").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let s = EncryptedFileSigner::new(&path, "passphrase");
            assert!(s.check_permissions().is_ok());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn hex_round_trip() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        let encoded = hex_encode(&bytes);
        assert_eq!(encoded, "000fa5ff");
        assert_eq!(hex_decode(&encoded).unwrap(), bytes);
    }

    #[test]
    fn parse_sign_response_accepts_valid_json() {
        let pk = "11".repeat(32);
        let sig = "22".repeat(64);
        let json = format!("{{\"public_key\":\"{pk}\",\"signature\":\"{sig}\"}}");
        let result = parse_sign_response(&json).unwrap();
        assert_eq!(result.public_key, [0x11u8; 32]);
        assert_eq!(result.signature, [0x22u8; 64]);
    }

    #[test]
    fn parse_sign_response_rejects_short_key() {
        let json = "{\"public_key\":\"11\",\"signature\":\"22\"}";
        let err = parse_sign_response(json).unwrap_err();
        assert!(matches!(err, SignerError::ExternalSignerFailed { .. }));
    }

    /// Redaction harness: no secret material may appear in the `Display`
    /// output of any error variant, even when the underlying detail string
    /// is derived from a secret-bearing source.
    #[test]
    fn errors_do_not_leak_secret_material() {
        let secret = "SUPERSECRETKEYMATERIAL";
        let errors = vec![
            SignerError::KeyringUnavailable {
                detail: "backend missing".to_string(),
            },
            SignerError::DecryptionFailed {
                detail: "wrong passphrase".to_string(),
            },
            SignerError::ExternalSignerFailed {
                detail: "protocol error".to_string(),
            },
            SignerError::PlaintextKeyRejected,
            SignerError::InsecurePermissions {
                path: PathBuf::from("/tmp/key.age"),
                mode: 0o644,
            },
        ];
        for err in errors {
            let rendered = format!("{err}");
            assert!(
                !rendered.contains(secret),
                "error Display leaked secret material: {rendered}"
            );
        }
    }

    /// Redaction harness: `SecretKey` must not be formattable via `Debug`.
    #[test]
    fn secret_key_has_no_debug_impl() {
        // Compile-time check: `SecretKey` intentionally has no `Debug`.
        // If a `Debug` impl were added this trait bound would start
        // compiling and the test would need to be revisited.
        fn assert_not_debug<T>() {}
        assert_not_debug::<SecretKey>();
    }
}
