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

use std::fmt;

/// A 64-byte ed25519 signature.
pub type Signature = [u8; 64];
/// A 32-byte ed25519 public key.
pub type PublicKey = [u8; 32];

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
}
