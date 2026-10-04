//! Local, hash-chained operation audit log (`~/.lafiya/audit.jsonl`).
//!
//! Every mutating CLI command appends one record. Each record stores the
//! SHA-256 of the previous record, so deleting or editing any line breaks
//! the chain and `lafiya-cli audit verify` reports where. Arguments are
//! stored only as hashes and every string is passed through the redaction
//! layer, so the log never contains secrets.

use crate::logging::redact;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Hash of the (nonexistent) record before the first one.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditRecord {
    pub seq: u64,
    pub timestamp: u64,
    pub command: String,
    pub network: String,
    pub contract: String,
    pub function: String,
    /// SHA-256 of each argument value, never the value itself.
    pub arg_hashes: Vec<String>,
    /// Source identity name or public key (`G...`), never a secret.
    pub signer: Option<String>,
    pub tx_hash: Option<String>,
    pub outcome: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Fields supplied by a command; the chain fields are filled by [`append`].
pub struct Operation<'a> {
    pub command: &'a str,
    pub network: &'a str,
    pub contract: &'a str,
    pub function: &'a str,
    pub args: &'a [&'a str],
    pub signer: Option<&'a str>,
    pub tx_hash: Option<&'a str>,
    pub outcome: &'a str,
}

/// `LAFIYA_AUDIT_LOG` if set, otherwise `~/.lafiya/audit.jsonl`.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LAFIYA_AUDIT_LOG") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".lafiya").join("audit.jsonl")
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Hash over every field except `hash` itself, in a fixed order.
fn record_hash(r: &AuditRecord) -> String {
    let mut unsealed = r.clone();
    unsealed.hash = String::new();
    sha256_hex(
        serde_json::to_string(&unsealed)
            .expect("record serializes")
            .as_bytes(),
    )
}

pub fn read_all(path: &Path) -> anyhow::Result<Vec<AuditRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| anyhow::anyhow!("line {}: {e}", i + 1)))
        .collect()
}

pub fn append(path: &Path, op: &Operation<'_>) -> anyhow::Result<AuditRecord> {
    let existing = read_all(path)?;
    let (seq, prev_hash) = existing
        .last()
        .map_or((0, GENESIS.to_string()), |r| (r.seq + 1, r.hash.clone()));
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut record = AuditRecord {
        seq,
        timestamp,
        command: redact(op.command),
        network: redact(op.network),
        contract: redact(op.contract),
        function: redact(op.function),
        arg_hashes: op.args.iter().map(|a| sha256_hex(a.as_bytes())).collect(),
        signer: op.signer.map(redact),
        tx_hash: op.tx_hash.map(str::to_string),
        outcome: redact(op.outcome),
        prev_hash,
        hash: String::new(),
    };
    record.hash = record_hash(&record);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    Ok(record)
}

/// Check every record's own hash and its link to the previous record.
/// Returns the number of verified records, or the first broken `seq`.
pub fn verify(records: &[AuditRecord]) -> Result<usize, String> {
    let mut prev = GENESIS.to_string();
    for (i, r) in records.iter().enumerate() {
        if r.seq != i as u64 {
            return Err(format!("record {i}: expected seq {i}, found {}", r.seq));
        }
        if r.prev_hash != prev {
            return Err(format!(
                "record {i}: prev_hash does not match the previous record"
            ));
        }
        if record_hash(r) != r.hash {
            return Err(format!(
                "record {i}: contents do not match its hash (edited?)"
            ));
        }
        prev = r.hash.clone();
    }
    Ok(records.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op<'a>(args: &'a [&'a str], outcome: &'a str) -> Operation<'a> {
        Operation {
            command: "attester add",
            network: "testnet",
            contract: "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            function: "add_attester",
            args,
            signer: Some("admin"),
            tx_hash: None,
            outcome,
        }
    }

    #[test]
    fn chain_verifies_after_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        for _ in 0..3 {
            append(&path, &op(&["GABC"], "success")).unwrap();
        }
        let records = read_all(&path).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].prev_hash, GENESIS);
        assert_eq!(records[1].prev_hash, records[0].hash);
        assert_eq!(verify(&records), Ok(3));
    }

    #[test]
    fn edited_record_breaks_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        append(&path, &op(&["GABC"], "success")).unwrap();
        append(&path, &op(&["GDEF"], "failure")).unwrap();
        let mut records = read_all(&path).unwrap();
        records[0].outcome = "failure".into();
        assert!(verify(&records).unwrap_err().contains("record 0"));
    }

    #[test]
    fn deleted_record_breaks_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        for _ in 0..3 {
            append(&path, &op(&["GABC"], "success")).unwrap();
        }
        let mut records = read_all(&path).unwrap();
        records.remove(1);
        assert!(verify(&records).is_err());
    }

    #[test]
    fn never_stores_secrets_or_raw_args() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let seed = "SBZVMB74Z76QZ3ZOY7UTDFYKMEGKW5XFJEB6PFKBF4UYSSWHG4EDH7PY";
        let mut o = op(
            &["GBZXN7PIRZGNMHGA7MUUUF4GWPY5AYPV6LY4UV2GL6VJGIQRXFDNMADI"],
            "success",
        );
        o.signer = Some(seed);
        append(&path, &o).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(seed));
        assert!(!raw.contains("GBZXN7PIRZGNMHGA7MUUUF4GWPY5AYPV6LY4UV2GL6VJGIQRXFDNMADI"));
        assert!(raw.contains("[REDACTED]"));
    }
}
