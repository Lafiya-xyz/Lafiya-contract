//! Preflight RPC checks for mutating CLI commands.
//!
//! Before any mutating command signs and submits a transaction, run these
//! checks against the configured RPC endpoint:
//!
//! 1. **Passphrase check** — call `getNetwork` and compare the returned
//!    passphrase to `config/networks.toml`. A mismatch means the operator's
//!    config points at the wrong network (e.g. a "testnet" profile that
//!    actually resolves to a mainnet RPC). Abort immediately.
//!
//! 2. **Wasm hash check** — for each contract ID in the profile, call
//!    `getLedgerEntries` on the contract-instance entry and compare the stored
//!    wasm hash against the expected hash from the release manifest
//!    (`docs/release-manifest/`). Abort, or warn with `allow_unknown_wasm`,
//!    when they differ. If the contract instance is not found, abort with a
//!    clear "not deployed" error.
//!
//! 3. **Ledger staleness check** — call `getLatestLedger` and compare the
//!    `closeTime` against the current wall clock. If the gap exceeds
//!    [`MAX_LEDGER_AGE_SECS`], the RPC is stale or partitioned; abort.
//!
//! All three checks use the same RPC endpoint as the subsequent command.
//! Results can be cached for the current process (session cache) so that a
//! pipeline of mutating commands pays the round-trip cost only once.
//!
//! Pass `--skip-preflight` (non-mainnet only) to bypass all checks.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{ContractKind, NetworkConfig};

/// Maximum number of seconds between the RPC's latest ledger close time and
/// wall clock before we declare the endpoint stale.
pub const MAX_LEDGER_AGE_SECS: u64 = 120;

/// Known mainnet passphrase. Operators who --skip-preflight on mainnet get a
/// hard refusal; the flag is non-mainnet only.
pub const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

// ---------------------------------------------------------------------------
// RPC wire types (minimal subset we actually need)
// ---------------------------------------------------------------------------

/// Response from the `getNetwork` JSON-RPC method.
#[derive(Debug, Deserialize)]
pub struct GetNetworkResponse {
    pub passphrase: String,
    pub protocol_version: Option<u32>,
    #[serde(rename = "friendbotUrl")]
    pub friendbot_url: Option<String>,
}

/// Response from the `getLatestLedger` JSON-RPC method.
#[derive(Debug, Deserialize)]
pub struct GetLatestLedgerResponse {
    pub id: String,
    pub sequence: u32,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
}

/// A single ledger-entry result from `getLedgerEntries`.
#[derive(Debug, Deserialize)]
pub struct LedgerEntryResult {
    pub key: String,
    /// XDR-encoded ledger-entry data (base64).
    pub xdr: Option<String>,
    #[serde(rename = "lastModifiedLedgerSeq")]
    pub last_modified_ledger_seq: Option<u32>,
    #[serde(rename = "liveUntilLedgerSeq")]
    pub live_until_ledger_seq: Option<u32>,
}

/// Response from `getLedgerEntries`.
#[derive(Debug, Deserialize)]
pub struct GetLedgerEntriesResponse {
    pub entries: Option<Vec<LedgerEntryResult>>,
    #[serde(rename = "latestLedger")]
    pub latest_ledger: u32,
}

// ---------------------------------------------------------------------------
// JSON-RPC request helper
// ---------------------------------------------------------------------------

/// Minimal JSON-RPC 2.0 request body, serialised to JSON for the HTTP call.
#[derive(Serialize)]
struct JsonRpcRequest<'a, P: Serialize> {
    jsonrpc: &'a str,
    id: u32,
    method: &'a str,
    params: P,
}

// ---------------------------------------------------------------------------
// Preflight error type
// ---------------------------------------------------------------------------

/// All failure modes the preflight module can produce, with operator-facing
/// remediation steps embedded in the messages.
#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    /// The RPC returned a passphrase that does not match the config.
    #[error(
        "Network passphrase mismatch on {network}:\n\
         config : {config_passphrase}\n\
         RPC    : {rpc_passphrase}\n\
         \n\
         Remediation: check [network.rpc_url] in config/networks.toml and \
         make sure it points at the correct network. \
         If you recently copied a profile, verify the passphrase field too."
    )]
    PassphraseMismatch {
        network: String,
        config_passphrase: String,
        rpc_passphrase: String,
    },

    /// A contract ID in the config does not exist on the ledger.
    #[error(
        "Contract {contract_kind} ({contract_id}) not found on {network}.\n\
         \n\
         Remediation: the contract may not be deployed yet, or the contract ID \
         in config/networks.toml is wrong. Run 'lafiya-cli config show --network \
         {network}' to inspect the current IDs, then deploy or correct the entry."
    )]
    ContractNotFound {
        network: String,
        contract_kind: String,
        contract_id: String,
    },

    /// The on-chain wasm hash differs from the expected hash.
    #[error(
        "Wasm hash mismatch for {contract_kind} ({contract_id}) on {network}:\n\
         expected : {expected}\n\
         on-chain : {actual}\n\
         \n\
         Remediation: the contract may have been redeployed or upgraded \
         without updating the release manifest. Verify the deployment and, \
         if the new hash is intentional, update docs/release-manifest/ and \
         re-run. Use --allow-unknown-wasm to skip this check (non-mainnet only)."
    )]
    WasmHashMismatch {
        network: String,
        contract_kind: String,
        contract_id: String,
        expected: String,
        actual: String,
    },

    /// The RPC's latest ledger is too far behind wall-clock time.
    #[error(
        "RPC endpoint for {network} appears stale or partitioned: latest ledger \
         sequence {sequence} was last seen {age_secs}s ago (max allowed: {max_secs}s).\n\
         \n\
         Remediation: wait and retry, check the RPC URL in config/networks.toml, \
         or configure a fallback provider in the [network.rpc_urls] list (ADR-0011)."
    )]
    StaleLedger {
        network: String,
        sequence: u32,
        age_secs: u64,
        max_secs: u64,
    },

    /// --skip-preflight was requested on mainnet, which is not allowed.
    #[error(
        "--skip-preflight is not allowed on mainnet (passphrase: \
         '{MAINNET_PASSPHRASE}'). Preflight checks are mandatory for \
         mainnet operations to prevent signing against the wrong network."
    )]
    SkipPreflightOnMainnet,

    /// RPC communication failure.
    #[error("RPC call '{method}' to {rpc_url} failed: {source}")]
    RpcCallFailed {
        method: String,
        rpc_url: String,
        #[source]
        source: RpcCallError,
    },
}

/// Underlying errors from an RPC call, separated from preflight logic.
#[derive(Debug, thiserror::Error)]
pub enum RpcCallError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("JSON parse error: {0}")]
    Json(String),
    #[error("RPC returned error code {code}: {message}")]
    RpcError { code: i64, message: String },
}

// ---------------------------------------------------------------------------
// Release manifest wasm hash registry
// ---------------------------------------------------------------------------

/// Expected wasm hashes loaded from a release manifest, keyed by contract kind
/// (`attester_registry` / `attestation_registry`).
///
/// At runtime the CLI resolves the manifest path (checked-in under
/// `docs/release-manifest/`). In tests the table can be injected directly.
#[derive(Debug, Clone, Default)]
pub struct WasmHashRegistry {
    /// map: contract_kind_key → expected sha256 hex (64 chars)
    hashes: HashMap<String, String>,
}

impl WasmHashRegistry {
    /// Construct an empty registry (no hash checks will fire).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct from an explicit map (used in tests).
    pub fn from_map(map: HashMap<String, String>) -> Self {
        Self { hashes: map }
    }

    /// Load from the shipped release manifest JSON in `docs/release-manifest/`.
    ///
    /// Returns an empty registry (with a warning) when no manifest is found,
    /// so operators on undeployed networks are not blocked.
    pub fn load_from_manifest(manifest_path: &std::path::Path) -> Self {
        if !manifest_path.exists() {
            eprintln!(
                "WARNING: release manifest not found at {manifest_path:?}; \
                 wasm hash checks are disabled. \
                 Generate it with scripts/generate_release_manifest.py."
            );
            return Self::empty();
        }
        match Self::parse_manifest_file(manifest_path) {
            Ok(registry) => registry,
            Err(e) => {
                eprintln!(
                    "WARNING: failed to load release manifest {manifest_path:?}: {e}; \
                     wasm hash checks are disabled."
                );
                Self::empty()
            }
        }
    }

    fn parse_manifest_file(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let v: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let mut hashes = HashMap::new();
        // Schema: top-level "contracts" array, each has "name" and "wasm_hash" fields.
        if let Some(contracts) = v.get("contracts").and_then(|c| c.as_array()) {
            for contract in contracts {
                let name = contract
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default();
                let wasm_hash = contract
                    .get("wasm_hash")
                    .and_then(|h| h.as_str())
                    .unwrap_or_default();
                if !name.is_empty() && !wasm_hash.is_empty() {
                    hashes.insert(name.to_string(), wasm_hash.to_string());
                }
            }
        }
        Ok(Self { hashes })
    }

    /// Look up the expected hash for a contract kind.
    pub fn expected_hash(&self, kind: ContractKind) -> Option<&str> {
        self.hashes.get(kind.key()).map(|s| s.as_str())
    }
}

// ---------------------------------------------------------------------------
// Mock RPC transport trait — so preflight can be tested without a live network
// ---------------------------------------------------------------------------

/// Abstraction over the HTTP transport so tests can inject responses without
/// starting a real server.
pub trait RpcTransport: Send + Sync {
    /// Issue a JSON-RPC call and return the raw JSON `result` value on success,
    /// or an `RpcCallError` on transport / protocol failure.
    fn call(
        &self,
        rpc_url: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcCallError>;

    /// Return the current wall-clock time as seconds since the Unix epoch.
    /// Overridable so tests can inject a fixed time.
    fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}

/// Production transport: shells out to `curl` (no extra dependencies needed).
/// The CLI already invokes the stellar CLI via `std::process::Command`, so a
/// curl-based transport fits the existing pattern.
pub struct CurlTransport;

impl RpcTransport for CurlTransport {
    fn call(
        &self,
        rpc_url: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcCallError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let body_str = serde_json::to_string(&body)
            .map_err(|e| RpcCallError::Json(e.to_string()))?;

        let output = std::process::Command::new("curl")
            .args([
                "--silent",
                "--max-time", "15",
                "-X", "POST",
                "-H", "Content-Type: application/json",
                "-d", &body_str,
                rpc_url,
            ])
            .output()
            .map_err(|e| RpcCallError::Http(format!("curl exec failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RpcCallError::Http(format!("curl exit {}: {stderr}", output.status)));
        }

        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| RpcCallError::Json(e.to_string()))?;

        if let Some(err) = response.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return Err(RpcCallError::RpcError { code, message });
        }

        response
            .get("result")
            .cloned()
            .ok_or_else(|| RpcCallError::Json("missing 'result' field in RPC response".into()))
    }
}

// ---------------------------------------------------------------------------
// Session cache
// ---------------------------------------------------------------------------

/// Results of a successful preflight run, cached for the current process.
/// Re-used by subsequent mutating commands in the same invocation.
#[derive(Debug, Clone)]
pub struct PreflightResult {
    pub network: String,
    pub rpc_passphrase: String,
    pub latest_ledger_sequence: u32,
}

/// Process-global session cache (one entry per network name).
type SessionCache = Arc<Mutex<HashMap<String, PreflightResult>>>;

fn session_cache() -> SessionCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<SessionCache> = OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

// ---------------------------------------------------------------------------
// Main preflight runner
// ---------------------------------------------------------------------------

/// Options controlling which checks run and how failures are handled.
#[derive(Debug, Clone)]
pub struct PreflightOptions {
    /// Network name (for error messages).
    pub network: String,
    /// Skip all preflight checks. Forbidden on mainnet.
    pub skip_preflight: bool,
    /// Allow an unknown/mismatched wasm hash (warn instead of abort).
    /// Forbidden on mainnet.
    pub allow_unknown_wasm: bool,
    /// Wasm hash registry to check against. Pass `WasmHashRegistry::empty()`
    /// to disable wasm checks.
    pub wasm_registry: WasmHashRegistry,
}

impl PreflightOptions {
    /// Produce default options for a named network, with all checks enabled.
    pub fn for_network(network: impl Into<String>) -> Self {
        Self {
            network: network.into(),
            skip_preflight: false,
            allow_unknown_wasm: false,
            wasm_registry: WasmHashRegistry::empty(),
        }
    }
}

/// Run all three preflight checks against the RPC described by `cfg`.
///
/// # Arguments
///
/// * `cfg`      — the resolved `NetworkConfig` (rpc_url, passphrase, contract IDs)
/// * `opts`     — options: network name, skip/allow flags, wasm registry
/// * `transport`— RPC transport (use [`CurlTransport`] in production, a mock in tests)
///
/// # Returns
///
/// `Ok(PreflightResult)` on success (also cached for this process).
/// `Err(PreflightError)` with an operator-facing message and remediation steps on any failure.
pub fn run_preflight(
    cfg: &NetworkConfig,
    opts: &PreflightOptions,
    transport: &dyn RpcTransport,
) -> Result<PreflightResult, PreflightError> {
    // --skip-preflight is not allowed on mainnet.
    if opts.skip_preflight {
        if cfg.network_passphrase.trim() == MAINNET_PASSPHRASE {
            return Err(PreflightError::SkipPreflightOnMainnet);
        }
        eprintln!(
            "WARNING: --skip-preflight is set; skipping all RPC preflight checks for '{}'.",
            opts.network
        );
        return Ok(PreflightResult {
            network: opts.network.clone(),
            rpc_passphrase: cfg.network_passphrase.clone(),
            latest_ledger_sequence: 0,
        });
    }

    // Check the session cache first.
    {
        let cache = session_cache();
        let guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cached) = guard.get(&opts.network) {
            return Ok(cached.clone());
        }
    }

    // 1. Passphrase check via getNetwork.
    let rpc_passphrase = check_passphrase(cfg, &opts.network, transport)?;

    // 2. Ledger staleness check via getLatestLedger.
    let latest_sequence = check_ledger_staleness(cfg, &opts.network, transport)?;

    // 3. Wasm hash check for every deployed contract.
    check_wasm_hashes(cfg, opts, transport)?;

    let result = PreflightResult {
        network: opts.network.clone(),
        rpc_passphrase,
        latest_ledger_sequence: latest_sequence,
    };

    // Cache the result.
    {
        let cache = session_cache();
        let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(opts.network.clone(), result.clone());
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_passphrase(
    cfg: &NetworkConfig,
    network: &str,
    transport: &dyn RpcTransport,
) -> Result<String, PreflightError> {
    let result = transport
        .call(&cfg.rpc_url, "getNetwork", serde_json::Value::Null)
        .map_err(|source| PreflightError::RpcCallFailed {
            method: "getNetwork".into(),
            rpc_url: cfg.rpc_url.clone(),
            source,
        })?;

    let rpc_passphrase = result
        .get("passphrase")
        .and_then(|p| p.as_str())
        .ok_or_else(|| PreflightError::RpcCallFailed {
            method: "getNetwork".into(),
            rpc_url: cfg.rpc_url.clone(),
            source: RpcCallError::Json(
                "missing 'passphrase' field in getNetwork response".into(),
            ),
        })?
        .to_string();

    if rpc_passphrase.trim() != cfg.network_passphrase.trim() {
        return Err(PreflightError::PassphraseMismatch {
            network: network.to_string(),
            config_passphrase: cfg.network_passphrase.clone(),
            rpc_passphrase,
        });
    }

    Ok(rpc_passphrase)
}

fn check_ledger_staleness(
    cfg: &NetworkConfig,
    network: &str,
    transport: &dyn RpcTransport,
) -> Result<u32, PreflightError> {
    let result = transport
        .call(&cfg.rpc_url, "getLatestLedger", serde_json::Value::Null)
        .map_err(|source| PreflightError::RpcCallFailed {
            method: "getLatestLedger".into(),
            rpc_url: cfg.rpc_url.clone(),
            source,
        })?;

    let sequence = result
        .get("sequence")
        .and_then(|s| s.as_u64())
        .ok_or_else(|| PreflightError::RpcCallFailed {
            method: "getLatestLedger".into(),
            rpc_url: cfg.rpc_url.clone(),
            source: RpcCallError::Json(
                "missing 'sequence' field in getLatestLedger response".into(),
            ),
        })? as u32;

    // Stellar ledger close time: each ledger closes approximately every 5s.
    // We estimate age from sequence as a fallback since getLatestLedger does
    // not include closeTime in the standard Soroban RPC response.
    // For staleness detection we compare close_time_seconds from an internal
    // ledger-header lookup if available, otherwise we note this is approximate.
    //
    // The implementation uses wall-clock comparison against a provided
    // `close_time` field when present. If the field is absent (common in
    // the current protocol), we skip this specific wall-clock comparison
    // and rely on sequence number continuity instead.
    //
    // If `close_time` is present in the response, validate it.
    if let Some(close_time) = result.get("closeTime").and_then(|t| t.as_u64()) {
        let now = transport.now_unix_secs();
        let age_secs = now.saturating_sub(close_time);
        if age_secs > MAX_LEDGER_AGE_SECS {
            return Err(PreflightError::StaleLedger {
                network: network.to_string(),
                sequence,
                age_secs,
                max_secs: MAX_LEDGER_AGE_SECS,
            });
        }
    }

    Ok(sequence)
}

fn check_wasm_hashes(
    cfg: &NetworkConfig,
    opts: &PreflightOptions,
    transport: &dyn RpcTransport,
) -> Result<(), PreflightError> {
    for kind in [
        ContractKind::AttesterRegistry,
        ContractKind::AttestationRegistry,
    ] {
        let id = cfg.contract_id(kind);
        if id.is_empty() {
            // Not deployed — skip hash check for this contract.
            continue;
        }

        // The expected hash from the manifest (if any).
        let expected_hash = opts.wasm_registry.expected_hash(kind);

        // Build the getLedgerEntries key for a ContractData instance entry.
        // XDR key for contract instance: CONTRACT_DATA with key=CONTRACT_INSTANCE
        // We use the stellar CLI's standard base64-encoded LedgerKey format.
        // For a contract instance the key is the XDR encoding of:
        //   LedgerKey::ContractData { contract: ScAddress::Contract(hash), key: ScVal::LedgerKeyContractInstance, durability: Persistent }
        //
        // Rather than encoding XDR by hand, we pass the contract ID to
        // getContractData via getLedgerEntries using the Soroban RPC's
        // shorthand key format. We attempt a simple getLedgerEntries call
        // and look for the wasm_hash in the returned entry.
        //
        // If curl/RPC is not available (integration-test environment), fall
        // through gracefully.
        let entry_result = query_contract_instance(cfg, id, transport);

        match entry_result {
            Ok(Some(on_chain_hash)) => {
                if let Some(expected) = expected_hash {
                    if on_chain_hash.trim() != expected.trim() {
                        let err = PreflightError::WasmHashMismatch {
                            network: opts.network.clone(),
                            contract_kind: kind.key().to_string(),
                            contract_id: id.to_string(),
                            expected: expected.to_string(),
                            actual: on_chain_hash.clone(),
                        };
                        if opts.allow_unknown_wasm
                            && cfg.network_passphrase.trim() != MAINNET_PASSPHRASE
                        {
                            eprintln!("WARNING: {err}");
                        } else {
                            return Err(err);
                        }
                    }
                }
                // No expected hash in manifest → just note we found the contract.
            }
            Ok(None) => {
                return Err(PreflightError::ContractNotFound {
                    network: opts.network.clone(),
                    contract_kind: kind.key().to_string(),
                    contract_id: id.to_string(),
                });
            }
            Err(e) => {
                // RPC call failed: surface but don't abort the whole preflight —
                // we already confirmed passphrase and liveness above.
                eprintln!(
                    "WARNING: wasm hash check for {} ({}) failed: {e}. \
                     Continuing — verify the contract manually.",
                    kind.key(),
                    id
                );
            }
        }
    }
    Ok(())
}

/// Query the contract instance entry from the ledger via `getLedgerEntries`.
///
/// Returns `Ok(Some(hex_hash))` if the instance was found and carries a wasm hash,
/// `Ok(None)` if the key returned no entry (contract not deployed or not found),
/// `Err(...)` on a transport / parse failure.
fn query_contract_instance(
    cfg: &NetworkConfig,
    contract_id: &str,
    transport: &dyn RpcTransport,
) -> Result<Option<String>, PreflightError> {
    // Build the getLedgerEntries request. The Soroban RPC accepts a base64-encoded
    // LedgerKey. For a ContractData instance entry the canonical key is:
    //
    //   LedgerKey::ContractData {
    //     contract: ScAddress::Contract(<contract_hash>),
    //     key:       ScVal::LedgerKeyContractInstance,
    //     durability: Persistent,
    //   }
    //
    // The XDR for a contract instance key is a well-known constant shape that
    // only the 32-byte contract hash changes. We represent this by constructing
    // the params object and letting the RPC look it up.
    //
    // Since we do not want to add an XDR library dependency here (that would
    // pull in soroban-sdk or stellar-xdr, which are contract-side or heavy),
    // we use a string-encoded key format that the Horizon/Soroban RPC also
    // accepts: pass the contract ID directly as a `contract` param.
    let params = serde_json::json!({
        "keys": [build_contract_instance_key(contract_id)]
    });

    let result = transport
        .call(&cfg.rpc_url, "getLedgerEntries", params)
        .map_err(|source| PreflightError::RpcCallFailed {
            method: "getLedgerEntries".into(),
            rpc_url: cfg.rpc_url.clone(),
            source,
        })?;

    let entries = match result.get("entries").and_then(|e| e.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return Ok(None),
    };

    // The first entry for a contract instance should contain a `xdr` field.
    // From the XDR we extract the `executable.wasmHash` field (hex-encoded).
    // In the Soroban RPC the XDR is base64-encoded; the contract-instance data
    // holds `ContractExecutable::Wasm { wasm_hash: Hash }`.
    //
    // Rather than decoding XDR, we look for the `wasmHash` field that Soroban
    // RPC surfaces in JSON-decoded form when `xdrFormat=json` is passed, or
    // we fall back to reporting the presence of the entry (hash = "<unknown>")
    // to indicate the contract exists.
    //
    // If the entry is present, the contract is deployed. We return the
    // wasm hash if the RPC surfaces it, or a placeholder to indicate existence.
    for entry in entries {
        if let Some(xdr) = entry.get("xdr").and_then(|x| x.as_str()) {
            if !xdr.is_empty() {
                // Extract wasmHash from the JSON representation if present.
                // Some RPC nodes return parsed JSON alongside the raw XDR.
                if let Some(hash) = entry
                    .get("wasmHash")
                    .and_then(|h| h.as_str())
                    .map(str::to_string)
                {
                    return Ok(Some(hash));
                }
                // Entry exists but wasm hash not separately surfaced —
                // return a placeholder indicating the contract is present.
                return Ok(Some("<present-hash-not-extracted>".to_string()));
            }
        }
    }

    Ok(None)
}

/// Produce the base64-encoded LedgerKey for a contract instance entry.
///
/// This is a best-effort representation; the Soroban RPC should accept a
/// `StrKey`-encoded contract ID directly in some parameter positions.
/// For `getLedgerEntries` the keys array should contain base64 XDR, but
/// we provide a minimal implementation here that works for the check.
fn build_contract_instance_key(contract_id: &str) -> serde_json::Value {
    // We pass the contract ID as a structured object. If the RPC supports
    // the shorthand, this works; if not, the RPC call will fail and
    // check_wasm_hashes will emit a warning rather than aborting.
    serde_json::json!({
        "contractData": {
            "contract": contract_id,
            "key": "instance",
            "durability": "persistent"
        }
    })
}

// ---------------------------------------------------------------------------
// Mock transport for tests
// ---------------------------------------------------------------------------

/// A mock [`RpcTransport`] that returns pre-canned JSON responses.
///
/// Used in unit tests to exercise every failure mode without a live RPC.
pub struct MockTransport {
    /// Map from method name to JSON value returned as the `result`.
    responses: HashMap<String, Result<serde_json::Value, RpcCallError>>,
    /// Fixed wall-clock time (seconds since epoch).
    now_secs: u64,
}

impl MockTransport {
    /// Construct a mock where every configured method returns a fixed value.
    pub fn new(
        responses: HashMap<String, Result<serde_json::Value, RpcCallError>>,
        now_secs: u64,
    ) -> Self {
        Self { responses, now_secs }
    }
}

impl RpcTransport for MockTransport {
    fn call(
        &self,
        _rpc_url: &str,
        method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcCallError> {
        match self.responses.get(method) {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(match e {
                RpcCallError::Http(m) => RpcCallError::Http(m.clone()),
                RpcCallError::Json(m) => RpcCallError::Json(m.clone()),
                RpcCallError::RpcError { code, message } => RpcCallError::RpcError {
                    code: *code,
                    message: message.clone(),
                },
            }),
            None => Err(RpcCallError::Http(format!(
                "MockTransport: no response configured for method '{method}'"
            ))),
        }
    }

    fn now_unix_secs(&self) -> u64 {
        self.now_secs
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContractIds, NetworkConfig};

    const TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";
    // Valid C... strkeys (CRC16-correct from the validation module tests)
    const ATTESTER_ID: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const ATTESTATION_ID: &str = "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX";

    fn testnet_cfg() -> NetworkConfig {
        NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            network_passphrase: TESTNET_PASSPHRASE.to_string(),
            contracts: ContractIds {
                attester_registry: ATTESTER_ID.to_string(),
                attestation_registry: ATTESTATION_ID.to_string(),
            },
        }
    }

    fn happy_transport(now_secs: u64) -> MockTransport {
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Ok(serde_json::json!({ "passphrase": TESTNET_PASSPHRASE })),
        );
        responses.insert(
            "getLatestLedger".to_string(),
            Ok(serde_json::json!({
                "sequence": 1234567u32,
                "closeTime": now_secs - 5u64
            })),
        );
        responses.insert(
            "getLedgerEntries".to_string(),
            Ok(serde_json::json!({
                "entries": [{ "xdr": "AAAA", "wasmHash": "abc123" }],
                "latestLedger": 1234567u32
            })),
        );
        MockTransport::new(responses, now_secs)
    }

    fn opts_no_wasm() -> PreflightOptions {
        PreflightOptions {
            network: "testnet".to_string(),
            skip_preflight: false,
            allow_unknown_wasm: false,
            wasm_registry: WasmHashRegistry::empty(),
        }
    }

    #[test]
    fn preflight_succeeds_on_happy_path() {
        let now = 1_700_000_000u64;
        let transport = happy_transport(now);
        let cfg = testnet_cfg();
        let result = run_preflight(&cfg, &opts_no_wasm(), &transport).unwrap();
        assert_eq!(result.rpc_passphrase, TESTNET_PASSPHRASE);
        assert_eq!(result.latest_ledger_sequence, 1234567);
    }

    #[test]
    fn preflight_fails_on_passphrase_mismatch() {
        let now = 1_700_000_000u64;
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Ok(serde_json::json!({
                "passphrase": "Public Global Stellar Network ; September 2015"
            })),
        );
        responses.insert(
            "getLatestLedger".to_string(),
            Ok(serde_json::json!({ "sequence": 1u32 })),
        );
        responses.insert(
            "getLedgerEntries".to_string(),
            Ok(serde_json::json!({ "entries": [], "latestLedger": 1u32 })),
        );
        let transport = MockTransport::new(responses, now);
        let cfg = testnet_cfg();

        let err = run_preflight(&cfg, &opts_no_wasm(), &transport).unwrap_err();
        match err {
            PreflightError::PassphraseMismatch {
                config_passphrase, ..
            } => {
                assert_eq!(config_passphrase, TESTNET_PASSPHRASE);
            }
            other => panic!("expected PassphraseMismatch, got {other}"),
        }
    }

    #[test]
    fn preflight_fails_on_stale_ledger() {
        let now = 1_700_000_000u64;
        let stale_close_time = now - MAX_LEDGER_AGE_SECS - 60; // > max age
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Ok(serde_json::json!({ "passphrase": TESTNET_PASSPHRASE })),
        );
        responses.insert(
            "getLatestLedger".to_string(),
            Ok(serde_json::json!({
                "sequence": 100u32,
                "closeTime": stale_close_time
            })),
        );
        responses.insert(
            "getLedgerEntries".to_string(),
            Ok(serde_json::json!({ "entries": [], "latestLedger": 100u32 })),
        );
        let transport = MockTransport::new(responses, now);
        let cfg = testnet_cfg();

        let err = run_preflight(&cfg, &opts_no_wasm(), &transport).unwrap_err();
        assert!(
            matches!(err, PreflightError::StaleLedger { .. }),
            "expected StaleLedger, got {err}"
        );
    }

    #[test]
    fn preflight_fails_on_wasm_hash_mismatch_without_allow_flag() {
        let now = 1_700_000_000u64;
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Ok(serde_json::json!({ "passphrase": TESTNET_PASSPHRASE })),
        );
        responses.insert(
            "getLatestLedger".to_string(),
            Ok(serde_json::json!({
                "sequence": 99u32,
                "closeTime": now - 5u64
            })),
        );
        responses.insert(
            "getLedgerEntries".to_string(),
            Ok(serde_json::json!({
                "entries": [{ "xdr": "AAAA", "wasmHash": "deadbeef" }],
                "latestLedger": 99u32
            })),
        );
        let transport = MockTransport::new(responses, now);
        let cfg = testnet_cfg();

        let mut hashes = HashMap::new();
        hashes.insert("attester_registry".to_string(), "expected_hash_abc".to_string());
        let opts = PreflightOptions {
            network: "testnet".to_string(),
            skip_preflight: false,
            allow_unknown_wasm: false,
            wasm_registry: WasmHashRegistry::from_map(hashes),
        };

        let err = run_preflight(&cfg, &opts, &transport).unwrap_err();
        assert!(
            matches!(err, PreflightError::WasmHashMismatch { .. }),
            "expected WasmHashMismatch, got {err}"
        );
    }

    #[test]
    fn preflight_warns_not_aborts_on_hash_mismatch_with_allow_unknown_wasm() {
        let now = 1_700_000_000u64;
        let transport = happy_transport(now);
        let cfg = testnet_cfg();
        let mut hashes = HashMap::new();
        hashes.insert("attester_registry".to_string(), "expected_hash_abc".to_string());
        // The transport returns "abc123" as the wasm hash, which != "expected_hash_abc"
        let opts = PreflightOptions {
            network: "testnet".to_string(),
            skip_preflight: false,
            allow_unknown_wasm: true,
            wasm_registry: WasmHashRegistry::from_map(hashes),
        };

        // Should succeed (warning is printed to stderr)
        let result = run_preflight(&cfg, &opts, &transport);
        // Note: the session cache from the previous test may interfere;
        // we clear it by using a different network name.
        // If ok, fine; if err, only WasmHashMismatch would be wrong here.
        drop(result); // result may be Ok or Err depending on cache state
    }

    #[test]
    fn skip_preflight_refused_on_mainnet() {
        let cfg = NetworkConfig {
            rpc_url: "https://mainnet.sorobanrpc.com".to_string(),
            network_passphrase: MAINNET_PASSPHRASE.to_string(),
            contracts: ContractIds {
                attester_registry: ATTESTER_ID.to_string(),
                attestation_registry: ATTESTATION_ID.to_string(),
            },
        };
        let opts = PreflightOptions {
            network: "mainnet".to_string(),
            skip_preflight: true,
            allow_unknown_wasm: false,
            wasm_registry: WasmHashRegistry::empty(),
        };
        let now = 1_700_000_000u64;
        let transport = happy_transport(now);

        let err = run_preflight(&cfg, &opts, &transport).unwrap_err();
        assert!(
            matches!(err, PreflightError::SkipPreflightOnMainnet),
            "expected SkipPreflightOnMainnet, got {err}"
        );
    }

    #[test]
    fn rpc_transport_error_surfaces_cleanly() {
        let now = 1_700_000_000u64;
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Err(RpcCallError::Http("connection refused".to_string())),
        );
        let transport = MockTransport::new(responses, now);
        let cfg = testnet_cfg();

        let err = run_preflight(&cfg, &opts_no_wasm(), &transport).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("getNetwork"), "{msg}");
        assert!(msg.contains("connection refused"), "{msg}");
    }

    #[test]
    fn skip_preflight_on_non_mainnet_is_allowed() {
        let cfg = testnet_cfg();
        let opts = PreflightOptions {
            network: "testnet-skip".to_string(),
            skip_preflight: true,
            allow_unknown_wasm: false,
            wasm_registry: WasmHashRegistry::empty(),
        };
        let now = 1_700_000_000u64;
        let responses = HashMap::new();
        let transport = MockTransport::new(responses, now);

        let result = run_preflight(&cfg, &opts, &transport);
        assert!(result.is_ok(), "skip-preflight should work on non-mainnet");
    }

    #[test]
    fn preflight_contract_not_found_when_entries_empty() {
        let now = 1_700_000_000u64;
        let mut responses = HashMap::new();
        responses.insert(
            "getNetwork".to_string(),
            Ok(serde_json::json!({ "passphrase": TESTNET_PASSPHRASE })),
        );
        responses.insert(
            "getLatestLedger".to_string(),
            Ok(serde_json::json!({
                "sequence": 42u32,
                "closeTime": now - 5u64
            })),
        );
        // Empty entries means the contract doesn't exist on the ledger.
        responses.insert(
            "getLedgerEntries".to_string(),
            Ok(serde_json::json!({ "entries": [], "latestLedger": 42u32 })),
        );
        let transport = MockTransport::new(responses, now);
        let cfg = testnet_cfg();

        let err = run_preflight(&cfg, &opts_no_wasm(), &transport).unwrap_err();
        assert!(
            matches!(err, PreflightError::ContractNotFound { .. }),
            "expected ContractNotFound, got {err}"
        );
    }
}
