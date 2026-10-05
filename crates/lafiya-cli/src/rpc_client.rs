//! Native Soroban RPC client trait and stub implementation.
//!
//! This module replaces the `stellar contract invoke` subprocess shelling
//! with direct JSON-RPC calls to a Soroban RPC endpoint (issue #395).
//!
//! ## Architecture
//!
//! ```text
//! lafiya-cli
//!   └── RpcClient (trait)
//!         ├── NativeRpcClient   ← direct JSON-RPC (this module)
//!         └── StellarCliClient  ← subprocess fallback (--via-stellar-cli)
//! ```
//!
//! The [`RpcClient`] trait exposes exactly the operations the CLI needs:
//! - **simulate** a transaction to get the footprint and resource fees.
//! - **send** a signed transaction envelope.
//! - **poll** for the transaction outcome.
//! - **get ledger entries** for reading contract state without signing.
//! - **get network info** (network passphrase, current ledger).
//!
//! [`NativeRpcClient`] is the long-term target.  It is a stub right now;
//! the actual XDR-building and JSON-RPC transport will be added once
//! `stellar-xdr` and `reqwest` (or a similar HTTP client) are pinned as
//! workspace dependencies (see issue #395 for the evaluation of
//! `stellar-rpc-client` vs. a thin in-house client).
//!
//! [`StellarCliClient`] wraps `std::process::Command` to call `stellar
//! contract invoke` and is the behaviour that existed before this refactor.
//! It is activated by passing `--via-stellar-cli` and will be removed in
//! the next release.

use crate::signer::{SignResult, Signer, SignerError};
use std::fmt;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// The outcome of `simulateTransaction`.
#[derive(Debug, Clone)]
pub struct SimulateResult {
    /// XDR-encoded transaction with the footprint and resource fee assembled.
    pub assembled_xdr: String,
    /// Minimum resource fee (in stroops) from the simulation.
    pub min_resource_fee: u64,
    /// Human-readable summary of the invocation returned by the simulator.
    pub invocation_summary: Option<String>,
}

/// The outcome of `sendTransaction`.
#[derive(Debug, Clone)]
pub struct SendResult {
    /// The transaction hash (hex).
    pub tx_hash: String,
    /// The immediate status returned by the RPC node.
    pub status: SendStatus,
}

/// Immediate status from `sendTransaction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendStatus {
    /// Transaction accepted into the queue; poll for outcome.
    Pending,
    /// Transaction was a duplicate of one already in the queue.
    Duplicate,
    /// Transaction was rejected by the node immediately.
    Error { message: String },
}

/// The outcome of `getTransaction`.
#[derive(Debug, Clone)]
pub struct TxResult {
    pub tx_hash: String,
    pub status: TxStatus,
}

/// On-chain transaction outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxStatus {
    /// A ledger closed with this transaction included and it succeeded.
    Success { ledger: u32 },
    /// A ledger closed with this transaction but it failed.
    Failed { ledger: u32, detail: String },
    /// The node has not seen this transaction (may still be in-flight).
    NotFound,
}

/// Network identity info from `getNetwork`.
#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub network_passphrase: String,
    pub protocol_version: u32,
    pub latest_ledger: u32,
}

/// A single Soroban ledger entry (raw XDR).
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub key_xdr: String,
    pub value_xdr: String,
    pub last_modified_ledger: u32,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur in the RPC client.
#[derive(Debug)]
pub enum RpcClientError {
    /// The HTTP request failed or could not be constructed.
    Transport { detail: String },
    /// The RPC server returned an unexpected response.
    Protocol { detail: String },
    /// XDR encoding/decoding failed.
    Xdr { detail: String },
    /// Signing failed (delegated from [`SignerError`]).
    Signing(SignerError),
    /// A transaction was submitted but polling timed out before confirmation.
    PollTimeout { tx_hash: String },
    /// The `stellar` subprocess failed (only for [`StellarCliClient`]).
    StellarCli { detail: String },
    /// Feature not yet implemented in this stub.
    NotImplemented { detail: String },
}

impl fmt::Display for RpcClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcClientError::Transport { detail } => write!(f, "RPC transport error: {detail}"),
            RpcClientError::Protocol { detail } => write!(f, "RPC protocol error: {detail}"),
            RpcClientError::Xdr { detail } => write!(f, "XDR error: {detail}"),
            RpcClientError::Signing(e) => write!(f, "signing error: {e}"),
            RpcClientError::PollTimeout { tx_hash } => {
                write!(f, "timed out polling for transaction {tx_hash}")
            }
            RpcClientError::StellarCli { detail } => write!(f, "stellar CLI error: {detail}"),
            RpcClientError::NotImplemented { detail } => {
                write!(f, "not yet implemented: {detail}")
            }
        }
    }
}

impl std::error::Error for RpcClientError {}

impl From<SignerError> for RpcClientError {
    fn from(e: SignerError) -> Self {
        RpcClientError::Signing(e)
    }
}

// ---------------------------------------------------------------------------
// RpcClient trait
// ---------------------------------------------------------------------------

/// The set of Soroban RPC operations the CLI needs.
///
/// Implementations must be `Send + Sync` so they can be used across async
/// boundaries once the native client becomes async.
pub trait RpcClient: Send + Sync {
    /// Simulate a transaction to obtain the footprint and resource fees.
    ///
    /// `unsigned_xdr` is the XDR-encoded unsigned `TransactionEnvelope`
    /// (with an empty `DecoratedSignature` list and no auth entries yet).
    fn simulate(
        &self,
        unsigned_xdr: &str,
    ) -> Result<SimulateResult, RpcClientError>;

    /// Submit a signed `TransactionEnvelope` and return the transaction hash
    /// and immediate status.
    fn send(
        &self,
        signed_xdr: &str,
    ) -> Result<SendResult, RpcClientError>;

    /// Poll `getTransaction` until the transaction reaches a terminal state
    /// or `max_attempts` is exhausted.
    fn poll(
        &self,
        tx_hash: &str,
        max_attempts: u32,
    ) -> Result<TxResult, RpcClientError>;

    /// Read ledger entries by their XDR-encoded keys.
    fn get_ledger_entries(
        &self,
        keys_xdr: &[&str],
    ) -> Result<Vec<LedgerEntry>, RpcClientError>;

    /// Fetch the network passphrase and latest ledger from the RPC node.
    fn get_network_info(&self) -> Result<NetworkInfo, RpcClientError>;

    /// High-level convenience: simulate → assemble → sign auth entries →
    /// sign envelope → send → poll.
    ///
    /// Callers that need fine-grained control (e.g. the multisig ceremony)
    /// call the lower-level methods individually.
    fn sign_and_submit(
        &self,
        unsigned_xdr: &str,
        signer: &dyn Signer,
        poll_attempts: u32,
    ) -> Result<TxResult, RpcClientError>;
}

// ---------------------------------------------------------------------------
// NativeRpcClient (stub — issue #395)
// ---------------------------------------------------------------------------

/// Direct Soroban JSON-RPC client.
///
/// **Current status:** stub.  Every method returns
/// `RpcClientError::NotImplemented`.  The transport layer (`reqwest` or a
/// minimal `ureq`-based client), the XDR builder (`stellar-xdr`), and the
/// actual JSON-RPC protocol will be filled in as part of the full issue #395
/// implementation.
///
/// The choice between `stellar-rpc-client` (the official crate published by
/// SDF) and a thin in-house client is documented in the spike referenced from
/// issue #395; this struct is written to be easily adapted to either approach
/// because the `RpcClient` trait boundary is stable.
pub struct NativeRpcClient {
    /// The Soroban RPC endpoint URL (e.g. `https://soroban-testnet.stellar.org`).
    pub rpc_url: String,
    /// Network passphrase (e.g. `Test SDF Network ; September 2015`).
    pub network_passphrase: String,
}

impl NativeRpcClient {
    pub fn new(rpc_url: impl Into<String>, network_passphrase: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            network_passphrase: network_passphrase.into(),
        }
    }
}

impl RpcClient for NativeRpcClient {
    fn simulate(&self, _unsigned_xdr: &str) -> Result<SimulateResult, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::simulate — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn send(&self, _signed_xdr: &str) -> Result<SendResult, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::send — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn poll(&self, _tx_hash: &str, _max_attempts: u32) -> Result<TxResult, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::poll — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn get_ledger_entries(
        &self,
        _keys_xdr: &[&str],
    ) -> Result<Vec<LedgerEntry>, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::get_ledger_entries — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn get_network_info(&self) -> Result<NetworkInfo, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::get_network_info — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn sign_and_submit(
        &self,
        _unsigned_xdr: &str,
        _signer: &dyn Signer,
        _poll_attempts: u32,
    ) -> Result<TxResult, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::sign_and_submit — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// StellarCliClient (--via-stellar-cli fallback)
// ---------------------------------------------------------------------------

/// Fallback RPC client that delegates to the `stellar` binary.
///
/// This is used when `--via-stellar-cli` is passed.  It reproduces the
/// behaviour that existed before the native RPC client work, so operators
/// can continue to use their existing stellar-cli setup during the
/// transition period.
///
/// **Deprecation notice:** this struct will be removed in the release after
/// every operator has migrated to the native client.  A warning is printed
/// on stderr whenever a command routes through this path.
pub struct StellarCliClient {
    pub rpc_url: String,
    pub network_passphrase: String,
}

impl StellarCliClient {
    pub fn new(rpc_url: impl Into<String>, network_passphrase: impl Into<String>) -> Self {
        eprintln!(
            "WARNING: --via-stellar-cli is deprecated and will be removed in a future release. \
             Once the native Soroban RPC client is complete, remove --via-stellar-cli."
        );
        Self {
            rpc_url: rpc_url.into(),
            network_passphrase: network_passphrase.into(),
        }
    }
}

impl RpcClient for StellarCliClient {
    fn simulate(&self, _unsigned_xdr: &str) -> Result<SimulateResult, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "simulate is not available via the stellar CLI subprocess path".to_string(),
        })
    }

    fn send(&self, _signed_xdr: &str) -> Result<SendResult, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "send is not directly available via the stellar CLI subprocess path; \
                     use sign_and_submit"
                .to_string(),
        })
    }

    fn poll(&self, _tx_hash: &str, _max_attempts: u32) -> Result<TxResult, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "poll is not available via the stellar CLI subprocess path".to_string(),
        })
    }

    fn get_ledger_entries(
        &self,
        _keys_xdr: &[&str],
    ) -> Result<Vec<LedgerEntry>, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "get_ledger_entries is not available via the stellar CLI subprocess path"
                .to_string(),
        })
    }

    fn get_network_info(&self) -> Result<NetworkInfo, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "get_network_info is not directly available via the stellar CLI subprocess path"
                .to_string(),
        })
    }

    fn sign_and_submit(
        &self,
        _unsigned_xdr: &str,
        _signer: &dyn Signer,
        _poll_attempts: u32,
    ) -> Result<TxResult, RpcClientError> {
        // The subprocess call is handled by the CLI match arm that checks
        // `via_stellar_cli`.  This path is a no-op stub to satisfy the trait.
        Err(RpcClientError::StellarCli {
            detail: "sign_and_submit via stellar CLI is handled by the CLI command dispatch, \
                     not through the RpcClient trait"
                .to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Factory helper
// ---------------------------------------------------------------------------

/// Construct the appropriate [`RpcClient`] based on the `--via-stellar-cli`
/// flag.
///
/// When `via_stellar_cli` is `true`, emits a deprecation warning and returns
/// a [`StellarCliClient`].  Otherwise returns a [`NativeRpcClient`].
pub fn make_rpc_client(
    rpc_url: &str,
    network_passphrase: &str,
    via_stellar_cli: bool,
) -> Box<dyn RpcClient> {
    if via_stellar_cli {
        Box::new(StellarCliClient::new(rpc_url, network_passphrase))
    } else {
        Box::new(NativeRpcClient::new(rpc_url, network_passphrase))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::LocalSigner;

    #[test]
    fn native_client_returns_not_implemented() {
        let client = NativeRpcClient::new(
            "https://soroban-testnet.stellar.org",
            "Test SDF Network ; September 2015",
        );
        let err = client.simulate("fake_xdr").unwrap_err();
        assert!(
            matches!(err, RpcClientError::NotImplemented { .. }),
            "expected NotImplemented, got: {err}"
        );
    }

    #[test]
    fn make_rpc_client_native_path() {
        let client = make_rpc_client("https://example.org", "Test Net", false);
        // The native client returns NotImplemented, confirming the factory
        // chose the correct branch.
        let err = client.simulate("xdr").unwrap_err();
        assert!(matches!(err, RpcClientError::NotImplemented { .. }));
    }

    #[test]
    fn make_rpc_client_stellar_cli_path() {
        let client = make_rpc_client("https://example.org", "Test Net", true);
        // The stellar-cli client returns StellarCli error for simulate.
        let err = client.simulate("xdr").unwrap_err();
        assert!(matches!(err, RpcClientError::StellarCli { .. }));
    }

    #[test]
    fn poll_and_send_not_implemented_on_native() {
        let client = NativeRpcClient::new("https://example.org", "Test Net");
        assert!(matches!(
            client.send("xdr").unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
        assert!(matches!(
            client.poll("hash", 10).unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
        assert!(matches!(
            client.get_ledger_entries(&[]).unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
        assert!(matches!(
            client.get_network_info().unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
        let signer = LocalSigner::new("key");
        assert!(matches!(
            client.sign_and_submit("xdr", &signer, 5).unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
    }
}
