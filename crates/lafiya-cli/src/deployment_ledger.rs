//! Append-only, hash-chained deployment ledger (`deployments/<network>.jsonl`).
//!
//! `config/networks.toml` only ever holds the *current* contract ID per
//! network; it has no history. This module gives every deploy/initialize/
//! upgrade/migrate/admin_transfer/repoint event a durable record, chained by
//! hash so an edit or deletion of an earlier line is detectable. See
//! `deployments/README.md` and `deployments/schema.json` for the on-disk
//! format this mirrors exactly.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployEvent {
    Deploy,
    Initialize,
    Upgrade,
    Migrate,
    AdminTransfer,
    Repoint,
}

impl fmt::Display for DeployEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DeployEvent::Deploy => "deploy",
            DeployEvent::Initialize => "initialize",
            DeployEvent::Upgrade => "upgrade",
            DeployEvent::Migrate => "migrate",
            DeployEvent::AdminTransfer => "admin_transfer",
            DeployEvent::Repoint => "repoint",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for DeployEvent {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "deploy" => Ok(DeployEvent::Deploy),
            "initialize" => Ok(DeployEvent::Initialize),
            "upgrade" => Ok(DeployEvent::Upgrade),
            "migrate" => Ok(DeployEvent::Migrate),
            "admin_transfer" => Ok(DeployEvent::AdminTransfer),
            "repoint" => Ok(DeployEvent::Repoint),
            other => Err(format!(
                "invalid event '{other}' (expected one of: deploy, initialize, upgrade, migrate, admin_transfer, repoint)"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentRecord {
    pub event: DeployEvent,
    pub contract_kind: String,
    pub contract_id: String,
    pub wasm_sha256: Option<String>,
    #[serde(default)]
    pub previous_wasm_sha256: Option<String>,
    pub tx_hash: Option<String>,
    pub ledger: Option<u32>,
    pub timestamp: String,
    pub git_commit: String,
    pub release_version: String,
    pub operator: String,
    pub prev_record_sha256: Option<String>,
}

/// sha256 of the exact bytes that will be (or were) written as one line,
/// i.e. the record serialized without its own `prev_record_sha256` echoed
/// back into the hash -- otherwise every hash would depend on itself.
fn record_hash(record: &DeploymentRecord) -> Result<String, String> {
    let mut for_hash = record.clone();
    for_hash.prev_record_sha256 = None;
    let bytes = serde_json::to_vec(&for_hash).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn ledger_path(deployments_dir: &Path, network: &str) -> PathBuf {
    deployments_dir.join(format!("{network}.jsonl"))
}

fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("failed to read {path:?}: {e}"))?;
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect())
}

/// Append `record` to `deployments/<network>.jsonl`, filling in
/// `prev_record_sha256` from the file's current last line (`None` if the
/// file is empty or missing).
pub fn append(
    deployments_dir: &Path,
    network: &str,
    mut record: DeploymentRecord,
) -> Result<PathBuf, String> {
    let path = ledger_path(deployments_dir, network);
    let lines = read_lines(&path)?;

    record.prev_record_sha256 = match lines.last() {
        Some(last_line) => {
            let last: DeploymentRecord = serde_json::from_str(last_line)
                .map_err(|e| format!("corrupt last record in {path:?}: {e}"))?;
            Some(record_hash(&last)?)
        }
        None => None,
    };

    let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    fs::create_dir_all(deployments_dir).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("failed to open {path:?}: {e}"))?;
    writeln!(file, "{line}").map_err(|e| e.to_string())?;
    Ok(path)
}

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub records_checked: usize,
    pub errors: Vec<String>,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Severity of a single doctor check, ordered so the worst wins when
/// aggregating several findings into one status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Info,
    Warning,
    Blocking,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Severity::Ok => "ok",
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Blocking => "blocking",
        };
        f.write_str(s)
    }
}

/// One machine-readable finding from a doctor check: what was checked, how
/// bad it is, and what the operator should do about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorFinding {
    pub check: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl DoctorFinding {
    pub fn new(check: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            severity,
            message: message.into(),
            remediation: None,
        }
    }

    pub fn with_remediation(mut self, hint: impl Into<String>) -> Self {
        self.remediation = Some(hint.into());
        self
    }

    pub fn is_blocking(&self) -> bool {
        self.severity == Severity::Blocking
    }
}

/// The tip (most recent record) of a network's ledger, if any.
fn ledger_tip(deployments_dir: &Path, network: &str) -> Result<Option<DeploymentRecord>, String> {
    let path = ledger_path(deployments_dir, network);
    let lines = read_lines(&path)?;
    match lines.last() {
        Some(line) => serde_json::from_str(line)
            .map(Some)
            .map_err(|e| format!("corrupt last record in {path:?}: {e}")),
        None => Ok(None),
    }
}

/// Doctor check: does the configured contract ID / wasm hash still match the
/// deployment ledger's tip for this network?
///
/// A testnet reset (or a manual repoint) leaves `config/networks.toml`
/// pointing at a contract that the ledger says was superseded, or at a
/// contract whose recorded wasm hash no longer matches the build being
/// deployed. Both are blocking: a mutating action against a stale contract ID
/// either fails late or, worse, targets the wrong instance.
///
/// `configured_contract_id` and `configured_wasm_sha256` come from the
/// resolved network config; `configured_wasm_sha256` is optional because a
/// config may only pin the contract ID.
pub fn check_contract_drift(
    deployments_dir: &Path,
    network: &str,
    configured_contract_id: &str,
    configured_wasm_sha256: Option<&str>,
) -> Result<DoctorFinding, String> {
    let check = "deployment_ledger.contract_drift";
    let tip = match ledger_tip(deployments_dir, network)? {
        Some(tip) => tip,
        None => {
            return Ok(DoctorFinding::new(
                check,
                Severity::Warning,
                format!("no deployment ledger records for network '{network}'"),
            )
            .with_remediation(format!(
                "run `lafiya deploy --network {network}` to record the first deployment in deployments/{network}.jsonl"
            )));
        }
    };

    if tip.contract_id != configured_contract_id {
        return Ok(DoctorFinding::new(
            check,
            Severity::Blocking,
            format!(
                "configured contract id {configured_contract_id} does not match ledger tip {} (last {} at ledger {:?}) -- likely a stale config after a network reset or repoint",
                tip.contract_id, tip.event, tip.ledger
            ),
        )
        .with_remediation(format!(
            "re-run `lafiya deploy --network {network}` (or `lafiya repoint`) and update config/networks.toml to the new contract id"
        )));
    }

    if let (Some(configured), Some(recorded)) =
        (configured_wasm_sha256, tip.wasm_sha256.as_deref())
    {
        if configured != recorded {
            return Ok(DoctorFinding::new(
                check,
                Severity::Blocking,
                format!(
                    "configured wasm hash {configured} does not match ledger tip wasm hash {recorded} for {}",
                    tip.contract_id
                ),
            )
            .with_remediation(format!(
                "confirm the build matches the deployed artifact, or record the new deployment with `lafiya deploy --network {network}`"
            )));
        }
    }

    Ok(DoctorFinding::new(
        check,
        Severity::Ok,
        format!(
            "configured contract {} matches ledger tip (last {} at ledger {:?})",
            tip.contract_id, tip.event, tip.ledger
        ),
    ))
}

/// Cross-check the contract IDs a mutating operation is about to touch
/// against the deployment ledger for `network`, before anything is signed.
///
/// This is the guard rail that catches a mixed configuration -- e.g. a
/// `mainnet` profile whose `config/networks.toml` still carries testnet
/// contract IDs. Contract IDs are network-specific, so a configured ID that
/// the ledger for this network has never seen (or that the ledger says was
/// superseded by a repoint) is a blocking mismatch: signing would either
/// fail late or target the wrong instance.
///
/// `configured_contract_ids` are the IDs resolved from the active network
/// config for this operation. Returns one finding per mismatched ID; an
/// empty result means every ID is consistent with the ledger.
pub fn check_contract_ids_for_network(
    deployments_dir: &Path,
    network: &str,
    configured_contract_ids: &[&str],
) -> Result<Vec<DoctorFinding>, String> {
    let check = "deployment_ledger.network_contract_mismatch";
    let path = ledger_path(deployments_dir, network);
    let lines = read_lines(&path)?;

    // Latest record per contract ID, in file order, so a repoint supersedes
    // the ID it replaced.
    let mut latest_by_contract: std::collections::HashMap<String, DeploymentRecord> =
        std::collections::HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        let record: DeploymentRecord = serde_json::from_str(line)
            .map_err(|e| format!("corrupt record at line {} in {path:?}: {e}", i + 1))?;
        latest_by_contract.insert(record.contract_id.clone(), record);
    }

    let mut findings = Vec::new();
    for configured in configured_contract_ids {
        match latest_by_contract.get(*configured) {
            Some(record) if record.event == DeployEvent::Repoint => {
                findings.push(
                    DoctorFinding::new(
                        check,
                        Severity::Blocking,
                        format!(
                            "contract id {configured} was superseded by a repoint on network '{network}' (ledger {:?}) -- refusing to sign against a stale contract",
                            record.ledger
                        ),
                    )
                    .with_remediation(format!(
                        "update config/networks.toml to the contract id recorded by the repoint, or re-run `lafiya deploy --network {network}`"
                    )),
                );
            }
            Some(_) => {}
            None => {
                let known: Vec<&str> = latest_by_contract.keys().map(String::as_str).collect();
                findings.push(
                    DoctorFinding::new(
                        check,
                        Severity::Blocking,
                        format!(
                            "contract id {configured} is not recorded in the deployment ledger for network '{network}' -- the configuration looks mixed (e.g. a mainnet profile with testnet contract ids)"
                        ),
                    )
                    .with_remediation(if known.is_empty() {
                        format!(
                            "run `lafiya deploy --network {network}` to record the first deployment, or fix the active profile with `lafiya-cli profile use <name>`"
                        )
                    } else {
                        format!(
                            "use one of the contract ids recorded for '{network}' ({}) or fix the active profile with `lafiya-cli profile use <name>`",
                            known.join(", ")
                        )
                    }),
                );
            }
        }
    }

    Ok(findings)
}

/// Convenience wrapper for the submit path: returns `Err` with a
/// human-readable summary when any configured contract ID does not belong to
/// `network` according to the deployment ledger. Call this before signing.
pub fn ensure_contract_ids_match_network(
    deployments_dir: &Path,
    network: &str,
    configured_contract_ids: &[&str],
) -> Result<(), String> {
    let findings = check_contract_ids_for_network(deployments_dir, network, configured_contract_ids)?;
    if findings.is_empty() {
        return Ok(());
    }
    let details = findings
        .iter()
        .map(|f| f.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "refusing to sign: network/contract-id mismatch for '{network}': {details}"
    ))
}

/// Structural (offline) verification of a network's ledger file:
/// - the hash chain is intact (each record's `prev_record_sha256` matches
///   the previous line's hash);
/// - ledgers are monotonically non-decreasing per contract, when known;
/// - `deploy`/`upgrade`/`migrate` records carry a `wasm_sha256`.
///
/// This does not touch the network: confirming the chain's tip against
/// live on-chain state is a separate, RPC-backed check (see
/// `deployments/README.md`).
pub fn verify(deployments_dir: &Path, network: &str) -> Result<VerifyReport, String> {
    let path = ledger_path(deployments_dir, network);
    let lines = read_lines(&path)?;
    let mut report = VerifyReport::default();
    let mut prev_hash: Option<String> = None;
    let mut last_ledger_by_contract: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for (i, line) in lines.iter().enumerate() {
        let record: DeploymentRecord = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                report
                    .errors
                    .push(format!("line {}: invalid JSON record: {e}", i + 1));
                continue;
            }
        };

        if record.prev_record_sha256 != prev_hash {
            report.errors.push(format!(
                "line {}: prev_record_sha256 mismatch (chain broken -- an earlier line was edited, reordered, or deleted)",
                i + 1
            ));
        }

        if matches!(
            record.event,
            DeployEvent::Deploy | DeployEvent::Upgrade | DeployEvent::Migrate
        ) && record.wasm_sha256.is_none()
        {
            report.errors.push(format!(
                "line {}: {} record is missing wasm_sha256",
                i + 1,
                record.event
            ));
        }

        if let Some(ledger) = record.ledger {
            if let Some(&last) = last_ledger_by_contract.get(&record.contract_id) {
                if ledger < last {
                    report.errors.push(format!(
                        "line {}: ledger {} for {} is behind the previously recorded ledger {} -- history is not monotonic",
                        i + 1,
                        ledger,
                        record.contract_id,
                        last
                    ));
                }
            }
            last_ledger_by_contract.insert(record.contract_id.clone(), ledger);
        }

        prev_hash = match record_hash(&record) {
            Ok(h) => Some(h),
            Err(e) => {
                report
                    .errors
                    .push(format!("line {}: could not hash record: {e}", i + 1));
                None
            }
        };
        report.records_checked += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample(op: &str) -> DeploymentRecord {
        DeploymentRecord {
            event: DeployEvent::Deploy,
            contract_kind: "attester-registry".into(),
            contract_id: "CTESTCONTRACT".into(),
            wasm_sha256: Some("a".repeat(64)),
            previous_wasm_sha256: None,
            tx_hash: Some("tx1".into()),
            ledger: Some(100),
            timestamp: "2026-01-01T00:00:00Z".into(),
            git_commit: "deadbeef".into(),
            release_version: "0.1.0".into(),
            operator: op.into(),
            prev_record_sha256: None,
        }
    }

    #[test]
    fn append_chains_hashes_and_verify_passes() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();
        let mut second = sample("bob");
        second.event = DeployEvent::Upgrade;
        second.ledger = Some(150);
        append(dir.path(), "testnet", second).unwrap();

        let report = verify(dir.path(), "testnet").unwrap();
        assert!(report.is_ok(), "unexpected errors: {:?}", report.errors);
        assert_eq!(report.records_checked, 2);
    }

    #[test]
    fn verify_detects_tampered_line() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();
        append(dir.path(), "testnet", sample("bob")).unwrap();

        let path = ledger_path(dir.path(), "testnet");
        let mut lines: Vec<String> = read_lines(&path).unwrap();
        // Tamper with the first record's operator without recomputing the chain.
        let mut first: DeploymentRecord = serde_json::from_str(&lines[0]).unwrap();
        first.operator = "mallory".into();
        lines[0] = serde_json::to_string(&first).unwrap();
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let report = verify(dir.path(), "testnet").unwrap();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.contains("chain broken")));
    }

    #[test]
    fn verify_detects_non_monotonic_ledger() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();
        let mut second = sample("bob");
        second.event = DeployEvent::Upgrade;
        second.ledger = Some(50); // goes backwards from 100
        append(dir.path(), "testnet", second).unwrap();

        let report = verify(dir.path(), "testnet").unwrap();
        assert!(report.errors.iter().any(|e| e.contains("not monotonic")));
    }

    #[test]
    fn verify_empty_ledger_is_ok() {
        let dir = tempdir().unwrap();
        let report = verify(dir.path(), "testnet").unwrap();
        assert!(report.is_ok());
        assert_eq!(report.records_checked, 0);
    }

    #[test]
    fn drift_check_passes_when_config_matches_tip() {
        let dir = tempdir().unwrap();
        let record = sample("alice");
        let wasm = record.wasm_sha256.clone().unwrap();
        append(dir.path(), "testnet", record).unwrap();

        let finding =
            check_contract_drift(dir.path(), "testnet", "CTESTCONTRACT", Some(&wasm)).unwrap();
        assert_eq!(finding.severity, Severity::Ok);
        assert!(!finding.is_blocking());
        assert!(finding.remediation.is_none());
    }

    #[test]
    fn drift_check_blocks_on_stale_contract_id() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();

        let finding =
            check_contract_drift(dir.path(), "testnet", "CNEWCONTRACT", None).unwrap();
        assert_eq!(finding.severity, Severity::Blocking);
        assert!(finding.is_blocking());
        assert!(finding.message.contains("stale config"));
        assert!(finding.remediation.is_some());
    }

    #[test]
    fn drift_check_blocks_on_wasm_hash_mismatch() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();

        let finding = check_contract_drift(
            dir.path(),
            "testnet",
            "CTESTCONTRACT",
            Some(&"b".repeat(64)),
        )
        .unwrap();
        assert_eq!(finding.severity, Severity::Blocking);
        assert!(finding.message.contains("wasm hash"));
        assert!(finding.remediation.is_some());
    }

    #[test]
    fn drift_check_warns_when_ledger_missing() {
        let dir = tempdir().unwrap();
        let finding =
            check_contract_drift(dir.path(), "testnet", "CTESTCONTRACT", None).unwrap();
        assert_eq!(finding.severity, Severity::Warning);
        assert!(!finding.is_blocking());
        assert!(finding.remediation.is_some());
    }

    #[test]
    fn contract_ids_match_when_recorded_for_network() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();

        let findings =
            check_contract_ids_for_network(dir.path(), "testnet", &["CTESTCONTRACT"]).unwrap();
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
        ensure_contract_ids_match_network(dir.path(), "testnet", &["CTESTCONTRACT"]).unwrap();
    }

    #[test]
    fn contract_ids_mismatch_blocks_mixed_configuration() {
        let dir = tempdir().unwrap();
        // Ledger only knows about testnet's contract.
        append(dir.path(), "testnet", sample("alice")).unwrap();

        // A mainnet profile still pointing at the testnet contract id.
        let findings =
            check_contract_ids_for_network(dir.path(), "mainnet", &["CTESTCONTRACT"]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Blocking);
        assert!(findings[0].message.contains("mixed"));
        assert!(findings[0].remediation.is_some());

        let err = ensure_contract_ids_match_network(dir.path(), "mainnet", &["CTESTCONTRACT"])
            .unwrap_err();
        assert!(err.contains("refusing to sign"));
        assert!(err.contains("mainnet"));
    }

    #[test]
    fn contract_ids_mismatch_blocks_after_repoint() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();
        let mut repoint = sample("bob");
        repoint.event = DeployEvent::Repoint;
        repoint.contract_id = "CREPOINTED".into();
        append(dir.path(), "testnet", repoint).unwrap();

        let findings =
            check_contract_ids_for_network(dir.path(), "testnet", &["CTESTCONTRACT"]).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("superseded"));

        // The repointed-to id is fine.
        ensure_contract_ids_match_network(dir.path(), "testnet", &["CREPOINTED"]).unwrap();
    }

    #[test]
    fn contract_ids_mismatch_reports_every_bad_id() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();

        let findings = check_contract_ids_for_network(
            dir.path(),
            "testnet",
            &["CTESTCONTRACT", "COTHER", "CTHIRD"],
        )
        .unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.is_blocking()));
    }

    #[test]
    fn drift_check_uses_ledger_tip_not_first_record() {
        let dir = tempdir().unwrap();
        append(dir.path(), "testnet", sample("alice")).unwrap();
        let mut second = sample("bob");
        second.event = DeployEvent::Repoint;
        second.contract_id = "CREPOINTED".into();
        append(dir.path(), "testnet", second).unwrap();

        let finding =
            check_contract_drift(dir.path(), "testnet", "CTESTCONTRACT", None).unwrap();
        assert_eq!(finding.severity, Severity::Blocking);
        assert!(finding.message.contains("CREPOINTED"));
    }
}
