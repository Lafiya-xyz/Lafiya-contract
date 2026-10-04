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
    Ok(text.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect())
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
            let last: DeploymentRecord =
                serde_json::from_str(last_line).map_err(|e| format!("corrupt last record in {path:?}: {e}"))?;
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
                report.errors.push(format!("line {}: invalid JSON record: {e}", i + 1));
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
                report.errors.push(format!("line {}: could not hash record: {e}", i + 1));
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
}
