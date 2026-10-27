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

/// A single ledger key in the transaction footprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootprintKey {
    /// XDR-encoded `LedgerKey`.
    pub key_xdr: String,
    /// Best-effort human-readable rendering of the key (e.g. contract id +
    /// storage key), or `None` when it cannot be decoded.
    pub label: Option<String>,
}

/// The decoded ledger footprint of a simulated transaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LedgerFootprint {
    /// Keys the transaction reads.
    pub read_only: Vec<FootprintKey>,
    /// Keys the transaction reads and writes.
    pub read_write: Vec<FootprintKey>,
}

/// A single ledger entry that changes as a result of the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    /// XDR-encoded `LedgerKey` identifying the entry.
    pub key_xdr: String,
    /// Human-readable rendering of the key, when available.
    pub label: Option<String>,
    /// XDR-encoded entry value before the transaction, or `None` if the
    /// entry is being created.
    pub before_xdr: Option<String>,
    /// XDR-encoded entry value after the transaction, or `None` if the
    /// entry is being removed.
    pub after_xdr: Option<String>,
}

/// A balance change for a single account or trustline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceChange {
    /// The account (G...) or contract (C...) whose balance changes.
    pub account: String,
    /// Asset identifier (e.g. `native`, `CODE:ISSUER`).
    pub asset: String,
    /// Signed delta in stroops (or the asset's smallest unit).
    pub delta: i128,
}

/// A contract event emitted by the simulated transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEvent {
    /// The contract that emitted the event (C...).
    pub contract_id: String,
    /// The event topics, rendered as strings.
    pub topics: Vec<String>,
    /// The event body, rendered as a string.
    pub data: String,
}

/// The full, deterministic preview of a simulated transaction.
///
/// This is the data the CLI renders before asking the operator to confirm a
/// mutating submit (issue #536).  It is intentionally transport-agnostic so
/// it can be serialised to JSON for change-control tickets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimulationPreview {
    /// The decoded ledger footprint.
    pub footprint: LedgerFootprint,
    /// Ledger entries that change, with before/after values.
    pub state_changes: Vec<StateChange>,
    /// Balance changes for accounts and trustlines.
    pub balance_changes: Vec<BalanceChange>,
    /// Contract events emitted by the transaction.
    pub events: Vec<ContractEvent>,
    /// The assembled transaction XDR (footprint + resource fee applied).
    pub assembled_xdr: String,
    /// Minimum resource fee (in stroops) from the simulation.
    pub min_resource_fee: u64,
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
    /// The transaction could not be simulated.  Callers must fail closed and
    /// refuse to submit when this is returned (issue #536).
    SimulationFailed { detail: String },
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
            RpcClientError::SimulationFailed { detail } => {
                write!(f, "simulation failed: {detail}")
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

    /// Simulate a transaction and return the full preview (footprint, state
    /// changes, balance changes, and events) used to render the pre-submit
    /// diff (issue #536).
    ///
    /// Implementations must return [`RpcClientError::SimulationFailed`] when
    /// the simulation cannot be completed so callers can fail closed.
    fn simulate_preview(
        &self,
        unsigned_xdr: &str,
    ) -> Result<SimulationPreview, RpcClientError>;

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

    /// Query `getTransaction` once for the current status of `tx_hash`.
    ///
    /// Unlike [`RpcClient::poll`], this never retries: it returns
    /// [`TxStatus::NotFound`] when the node has not yet ingested the
    /// transaction.  Reconciliation uses this to distinguish "still
    /// in-flight" from a terminal outcome without blocking.
    fn get_transaction(&self, tx_hash: &str) -> Result<TxResult, RpcClientError>;

    /// Return the latest ledger sequence known to the RPC node.
    ///
    /// Reconciliation compares this against the submission intent's expiry
    /// ledger to prove that an ambiguous transaction can no longer land.
    fn latest_ledger(&self) -> Result<u32, RpcClientError>;

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

    fn simulate_preview(
        &self,
        _unsigned_xdr: &str,
    ) -> Result<SimulationPreview, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::simulate_preview — add stellar-xdr + HTTP client (issue #395)"
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

    fn get_transaction(&self, _tx_hash: &str) -> Result<TxResult, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::get_transaction — add stellar-xdr + HTTP client (issue #395)"
                .to_string(),
        })
    }

    fn latest_ledger(&self) -> Result<u32, RpcClientError> {
        Err(RpcClientError::NotImplemented {
            detail: "NativeRpcClient::latest_ledger — add stellar-xdr + HTTP client (issue #395)"
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

    fn simulate_preview(
        &self,
        _unsigned_xdr: &str,
    ) -> Result<SimulationPreview, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "simulate_preview is not available via the stellar CLI subprocess path"
                .to_string(),
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

    fn get_transaction(&self, _tx_hash: &str) -> Result<TxResult, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "get_transaction is not available via the stellar CLI subprocess path"
                .to_string(),
        })
    }

    fn latest_ledger(&self) -> Result<u32, RpcClientError> {
        Err(RpcClientError::StellarCli {
            detail: "latest_ledger is not available via the stellar CLI subprocess path"
                .to_string(),
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
// Doctor checks (issue #535)
// ---------------------------------------------------------------------------

/// Severity of a doctor check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorSeverity {
    /// Informational; never blocks.
    Info,
    /// Degraded but non-blocking.
    Warning,
    /// Blocking failure; causes a non-zero exit.
    Error,
}

impl DoctorSeverity {
    /// Stable machine-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            DoctorSeverity::Info => "info",
            DoctorSeverity::Warning => "warning",
            DoctorSeverity::Error => "error",
        }
    }

    /// Whether a result with this severity blocks a mutating action.
    pub fn is_blocking(&self) -> bool {
        matches!(self, DoctorSeverity::Error)
    }
}

/// The result of a single doctor check.
#[derive(Debug, Clone)]
pub struct DoctorCheck {
    /// Stable identifier (e.g. `rpc_health`).
    pub id: &'static str,
    /// Severity of the outcome.
    pub severity: DoctorSeverity,
    /// Short human-readable summary.
    pub message: String,
    /// Optional remediation hint for the operator.
    pub remediation: Option<String>,
}

impl DoctorCheck {
    fn new(
        id: &'static str,
        severity: DoctorSeverity,
        message: impl Into<String>,
        remediation: Option<&str>,
    ) -> Self {
        Self {
            id,
            severity,
            message: message.into(),
            remediation: remediation.map(str::to_string),
        }
    }

    /// Whether this check blocks a mutating action.
    pub fn is_blocking(&self) -> bool {
        self.severity.is_blocking()
    }
}

/// Minimum protocol version the CLI is known to work against.
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 20;

/// Default maximum ledger drift tolerated between the RPC node's latest
/// ledger and the ledger an instance entry was last modified at.
pub const DEFAULT_MAX_INSTANCE_LEDGER_DRIFT: u32 = 100_000;

/// Check that the RPC node is reachable and reports a supported protocol
/// version.
///
/// This performs a single `get_network_info` call, so it is bounded by the
/// provider's own transport timeout.
pub fn check_rpc_health(
    client: &dyn RpcClient,
    expected_passphrase: &str,
) -> DoctorCheck {
    match client.get_network_info() {
        Ok(info) => {
            if info.protocol_version < MIN_SUPPORTED_PROTOCOL_VERSION {
                return DoctorCheck::new(
                    "rpc_protocol_version",
                    DoctorSeverity::Error,
                    format!(
                        "RPC node reports protocol version {} (minimum supported is {})",
                        info.protocol_version, MIN_SUPPORTED_PROTOCOL_VERSION
                    ),
                    Some("Point --rpc-url at a node running a supported protocol version."),
                );
            }
            if info.network_passphrase != expected_passphrase {
                return DoctorCheck::new(
                    "rpc_network_passphrase",
                    DoctorSeverity::Error,
                    format!(
                        "RPC node passphrase {:?} does not match configured {:?}",
                        info.network_passphrase, expected_passphrase
                    ),
                    Some("Set --network-passphrase to match the target network."),
                );
            }
            DoctorCheck::new(
                "rpc_health",
                DoctorSeverity::Info,
                format!(
                    "RPC healthy at ledger {} (protocol {})",
                    info.latest_ledger, info.protocol_version
                ),
                None,
            )
        }
        Err(e) => DoctorCheck::new(
            "rpc_health",
            DoctorSeverity::Error,
            format!("RPC node unreachable: {e}"),
            Some("Check --rpc-url and network connectivity, then retry."),
        ),
    }
}

/// Check the remaining TTL of a contract instance entry.
///
/// `instance_key_xdr` is the XDR-encoded ledger key of the contract instance.
/// The check reads the entry's `last_modified_ledger` and compares it against
/// the node's latest ledger; if the drift exceeds `max_drift` the instance is
/// considered at risk of archival.
pub fn check_instance_ttl(
    client: &dyn RpcClient,
    instance_key_xdr: &str,
    max_drift: u32,
) -> DoctorCheck {
    let info = match client.get_network_info() {
        Ok(info) => info,
        Err(e) => {
            return DoctorCheck::new(
                "instance_ttl",
                DoctorSeverity::Error,
                format!("cannot evaluate instance TTL: {e}"),
                Some("Resolve the RPC health check first."),
            )
        }
    };

    let entries = match client.get_ledger_entries(&[instance_key_xdr]) {
        Ok(entries) => entries,
        Err(e) => {
            return DoctorCheck::new(
                "instance_ttl",
                DoctorSeverity::Error,
                format!("cannot read contract instance entry: {e}"),
                Some("Verify the contract id and that the instance is not archived."),
            )
        }
    };

    let entry = match entries.into_iter().next() {
        Some(entry) => entry,
        None => {
            return DoctorCheck::new(
                "instance_ttl",
                DoctorSeverity::Error,
                "contract instance entry not found on-chain".to_string(),
                Some("The contract may be archived or the contract id is stale."),
            )
        }
    };

    let drift = info.latest_ledger.saturating_sub(entry.last_modified_ledger);
    if drift > max_drift {
        DoctorCheck::new(
            "instance_ttl",
            DoctorSeverity::Error,
            format!(
                "contract instance last modified {} ledgers ago (limit {})",
                drift, max_drift
            ),
            Some("Extend the instance TTL before running mutating actions."),
        )
    } else if drift * 2 > max_drift {
        DoctorCheck::new(
            "instance_ttl",
            DoctorSeverity::Warning,
            format!(
                "contract instance last modified {} ledgers ago (limit {})",
                drift, max_drift
            ),
            Some("Consider extending the instance TTL soon."),
        )
    } else {
        DoctorCheck::new(
            "instance_ttl",
            DoctorSeverity::Info,
            format!("contract instance fresh ({} ledgers since last modification)", drift),
            None,
        )
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
    fn native_client_simulate_preview_not_implemented() {
        let client = NativeRpcClient::new("https://example.org", "Test Net");
        let err = client.simulate_preview("fake_xdr").unwrap_err();
        assert!(
            matches!(err, RpcClientError::NotImplemented { .. }),
            "expected NotImplemented, got: {err}"
        );
    }

    #[test]
    fn stellar_cli_simulate_preview_errors() {
        let client = StellarCliClient::new("https://example.org", "Test Net");
        let err = client.simulate_preview("fake_xdr").unwrap_err();
        assert!(matches!(err, RpcClientError::StellarCli { .. }));
    }

    #[test]
    fn simulation_failed_displays_distinctly() {
        let err = RpcClientError::SimulationFailed {
            detail: "boom".to_string(),
        };
        assert_eq!(err.to_string(), "simulation failed: boom");
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

    /// Mock provider for doctor checks.
    struct MockRpcClient {
        network: Result<NetworkInfo, RpcClientError>,
        entries: Result<Vec<LedgerEntry>, RpcClientError>,
    }

    impl RpcClient for MockRpcClient {
        fn simulate(&self, _unsigned_xdr: &str) -> Result<SimulateResult, RpcClientError> {
            Err(RpcClientError::NotImplemented { detail: "mock".into() })
        }
        fn send(&self, _signed_xdr: &str) -> Result<SendResult, RpcClientError> {
            Err(RpcClientError::NotImplemented { detail: "mock".into() })
        }
        fn poll(&self, _tx_hash: &str, _max_attempts: u32) -> Result<TxResult, RpcClientError> {
            Err(RpcClientError::NotImplemented { detail: "mock".into() })
        }
        fn get_ledger_entries(
            &self,
            _keys_xdr: &[&str],
        ) -> Result<Vec<LedgerEntry>, RpcClientError> {
            match &self.entries {
                Ok(entries) => Ok(entries.clone()),
                Err(_) => Err(RpcClientError::Transport { detail: "mock".into() }),
            }
        }
        fn get_network_info(&self) -> Result<NetworkInfo, RpcClientError> {
            match &self.network {
                Ok(info) => Ok(info.clone()),
                Err(_) => Err(RpcClientError::Transport { detail: "mock".into() }),
            }
        }
        fn sign_and_submit(
            &self,
            _unsigned_xdr: &str,
            _signer: &dyn Signer,
            _poll_attempts: u32,
        ) -> Result<TxResult, RpcClientError> {
            Err(RpcClientError::NotImplemented { detail: "mock".into() })
        }
    }

    fn healthy_network() -> NetworkInfo {
        NetworkInfo {
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            latest_ledger: 1_000,
        }
    }

    #[test]
    fn doctor_rpc_health_ok() {
        let client = MockRpcClient {
            network: Ok(healthy_network()),
            entries: Ok(vec![]),
        };
        let check = check_rpc_health(&client, "Test SDF Network ; September 2015");
        assert_eq!(check.severity, DoctorSeverity::Info);
        assert!(!check.is_blocking());
    }

    #[test]
    fn doctor_rpc_health_unreachable_is_blocking() {
        let client = MockRpcClient {
            network: Err(RpcClientError::Transport { detail: "down".into() }),
            entries: Ok(vec![]),
        };
        let check = check_rpc_health(&client, "Test SDF Network ; September 2015");
        assert_eq!(check.severity, DoctorSeverity::Error);
        assert!(check.is_blocking());
        assert!(check.remediation.is_some());
    }

    #[test]
    fn doctor_rpc_health_protocol_too_old_is_blocking() {
        let mut network = healthy_network();
        network.protocol_version = MIN_SUPPORTED_PROTOCOL_VERSION - 1;
        let client = MockRpcClient { network: Ok(network), entries: Ok(vec![]) };
        let check = check_rpc_health(&client, "Test SDF Network ; September 2015");
        assert_eq!(check.severity, DoctorSeverity::Error);
        assert_eq!(check.id, "rpc_protocol_version");
    }

    #[test]
    fn doctor_rpc_health_passphrase_mismatch_is_blocking() {
        let client = MockRpcClient {
            network: Ok(healthy_network()),
            entries: Ok(vec![]),
        };
        let check = check_rpc_health(&client, "Public Global Stellar Network ; September 2015");
        assert_eq!(check.severity, DoctorSeverity::Error);
        assert_eq!(check.id, "rpc_network_passphrase");
    }

    #[test]
    fn doctor_instance_ttl_fresh_is_info() {
        let client = MockRpcClient {
            network: Ok(healthy_network()),
            entries: Ok(vec![LedgerEntry {
                key_xdr: "key".into(),
                value_xdr: "value".into(),
                last_modified_ledger: 990,
            }]),
        };
        let check = check_instance_ttl(&client, "key", DEFAULT_MAX_INSTANCE_LEDGER_DRIFT);
        assert_eq!(check.severity, DoctorSeverity::Info);
    }

    #[test]
    fn doctor_instance_ttl_stale_is_blocking() {
        let client = MockRpcClient {
            network: Ok(healthy_network()),
            entries: Ok(vec![LedgerEntry {
                key_xdr: "key".into(),
                value_xdr: "value".into(),
                last_modified_ledger: 1,
            }]),
        };
        let check = check_instance_ttl(&client, "key", DEFAULT_MAX_INSTANCE_LEDGER_DRIFT);
        assert_eq!(check.severity, DoctorSeverity::Error);
        assert!(check.is_blocking());
    }

    #[test]
    fn doctor_instance_ttl_missing_entry_is_blocking() {
        let client = MockRpcClient {
            network: Ok(healthy_network()),
            entries: Ok(vec![]),
        };
        let check = check_instance_ttl(&client, "key", DEFAULT_MAX_INSTANCE_LEDGER_DRIFT);
        assert_eq!(check.severity, DoctorSeverity::Error);
    }

    #[test]
    fn reconciliation_ops_not_available_on_stellar_cli() {
        let client = make_rpc_client("https://example.org", "Test Net", true);
        assert!(matches!(
            client.get_transaction("hash").unwrap_err(),
            RpcClientError::StellarCli { .. }
        ));
        assert!(matches!(
            client.latest_ledger().unwrap_err(),
            RpcClientError::StellarCli { .. }
        ));
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
            client.get_transaction("hash").unwrap_err(),
            RpcClientError::NotImplemented { .. }
        ));
        assert!(matches!(
            client.latest_ledger().unwrap_err(),
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
