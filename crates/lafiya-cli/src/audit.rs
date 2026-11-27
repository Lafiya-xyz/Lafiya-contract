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
use std::collections::BTreeMap;
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

// ---------------------------------------------------------------------------
// Evidence bundle export / verify (issue #731)
// ---------------------------------------------------------------------------

/// Current evidence-bundle schema version.
pub const EVIDENCE_SCHEMA_VERSION: u32 = 1;

/// A single on-chain reference proving an item: the ledger it was read at and
/// the transaction hash that produced it (if any).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceRef {
    pub ledger: u32,
    pub tx_hash: Option<String>,
}

/// An attestation recorded for the record hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceAttestation {
    pub attester: String,
    pub record_hash: String,
    pub reference: EvidenceRef,
}

/// A change in an attester's status (added, removed, suspended, ...).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceAttesterStatus {
    pub attester: String,
    pub status: String,
    pub reference: EvidenceRef,
}

/// A dispute opened against the record hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceDispute {
    pub dispute_id: String,
    pub record_hash: String,
    pub reference: EvidenceRef,
}

/// A revocation of an attestation for the record hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceRevocation {
    pub attester: String,
    pub record_hash: String,
    pub reference: EvidenceRef,
}

/// Membership of the record hash in a status list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceStatusListMembership {
    pub list_id: String,
    pub record_hash: String,
    pub reference: EvidenceRef,
}

/// A self-contained, deterministic evidence bundle for one record hash.
///
/// Contains only hashes, addresses, ledger numbers and tx hashes — never PII.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceBundle {
    pub schema_version: u32,
    pub record_hash: String,
    pub ledger: u32,
    pub network: String,
    pub network_passphrase: String,
    pub contracts: Vec<String>,
    pub attestations: Vec<EvidenceAttestation>,
    pub attester_status_history: Vec<EvidenceAttesterStatus>,
    pub disputes: Vec<EvidenceDispute>,
    pub revocations: Vec<EvidenceRevocation>,
    pub status_list_membership: Vec<EvidenceStatusListMembership>,
}

impl EvidenceBundle {
    /// Sort every array into a stable order so the same ledger always yields
    /// byte-identical output.
    fn canonicalize(&mut self) {
        self.contracts.sort();
        self.contracts.dedup();
        self.attestations.sort();
        self.attester_status_history.sort();
        self.disputes.sort();
        self.revocations.sort();
        self.status_list_membership.sort();
    }

    /// Serialize to canonical JSON: sorted keys, no whitespace, stable arrays.
    pub fn to_canonical_json(&self) -> anyhow::Result<String> {
        let mut sorted = self.clone();
        sorted.canonicalize();
        let value = serde_json::to_value(&sorted)?;
        Ok(serde_json::to_string(&sort_keys(&value))?)
    }

    /// SHA-256 of each section, keyed by section name.
    pub fn section_hashes(&self) -> anyhow::Result<BTreeMap<String, String>> {
        let mut sorted = self.clone();
        sorted.canonicalize();
        let value = serde_json::to_value(&sorted)?;
        let mut out = BTreeMap::new();
        if let serde_json::Value::Object(map) = value {
            for (k, v) in map {
                out.insert(k, sha256_hex(serde_json::to_string(&v)?.as_bytes()));
            }
        }
        Ok(out)
    }
}

/// Recursively rebuild a JSON value with object keys sorted.
fn sort_keys(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), sort_keys(&map[k]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sort_keys).collect())
        }
        other => other.clone(),
    }
}

/// The manifest written alongside the bundle: per-section hashes plus an
/// optional operator signature over the manifest itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceManifest {
    pub schema_version: u32,
    pub record_hash: String,
    pub ledger: u32,
    pub section_hashes: BTreeMap<String, String>,
    pub signer: Option<String>,
    pub signature: Option<String>,
}

impl EvidenceManifest {
    /// Hash over the manifest contents excluding the signature.
    fn signing_payload(&self) -> anyhow::Result<String> {
        let mut unsigned = self.clone();
        unsigned.signature = None;
        let value = serde_json::to_value(&unsigned)?;
        Ok(serde_json::to_string(&sort_keys(&value))?)
    }
}

/// Collect the evidence bundle for `record_hash` at a pinned ledger.
///
/// `reads` supplies the contract reads (already fetched at `ledger`) and
/// `events` supplies the relevant events with their tx hashes. Both are
/// expected to be deterministic for a given ledger.
pub fn build_bundle(
    record_hash: &str,
    ledger: u32,
    network: &str,
    network_passphrase: &str,
    contracts: &[String],
    attestations: Vec<EvidenceAttestation>,
    attester_status_history: Vec<EvidenceAttesterStatus>,
    disputes: Vec<EvidenceDispute>,
    revocations: Vec<EvidenceRevocation>,
    status_list_membership: Vec<EvidenceStatusListMembership>,
) -> EvidenceBundle {
    let mut bundle = EvidenceBundle {
        schema_version: EVIDENCE_SCHEMA_VERSION,
        record_hash: record_hash.to_string(),
        ledger,
        network: network.to_string(),
        network_passphrase: network_passphrase.to_string(),
        contracts: contracts.to_vec(),
        attestations,
        attester_status_history,
        disputes,
        revocations,
        status_list_membership,
    };
    bundle.canonicalize();
    bundle
}

/// Export a bundle and its manifest to `out_path` (the bundle) and
/// `<out_path>.manifest.json` (the manifest). If `signer` is provided, the
/// manifest is signed with that operator key.
pub fn export(
    bundle: &EvidenceBundle,
    out_path: &Path,
    signer: Option<&crate::signer::Signer>,
) -> anyhow::Result<EvidenceManifest> {
    let json = bundle.to_canonical_json()?;
    let mut manifest = EvidenceManifest {
        schema_version: EVIDENCE_SCHEMA_VERSION,
        record_hash: bundle.record_hash.clone(),
        ledger: bundle.ledger,
        section_hashes: bundle.section_hashes()?,
        signer: signer.map(|s| s.public_key().to_string()),
        signature: None,
    };
    if let Some(s) = signer {
        let payload = manifest.signing_payload()?;
        manifest.signature = Some(s.sign_bytes(payload.as_bytes())?);
    }
    if let Some(dir) = out_path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(out_path, json.as_bytes())?;
    let manifest_path = manifest_path_for(out_path);
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    Ok(manifest)
}

fn manifest_path_for(bundle_path: &Path) -> PathBuf {
    let mut s = bundle_path.as_os_str().to_os_string();
    s.push(".manifest.json");
    PathBuf::from(s)
}

/// Verify a bundle against its manifest and (optionally) re-query RPC to
/// detect on-chain mismatches.
///
/// Returns `Ok(())` when the manifest hashes match the bundle contents and,
/// if `rpc` is provided, the pinned ledger still matches.
pub fn verify_bundle(
    bundle_path: &Path,
    rpc: Option<&crate::rpc::RpcClient>,
) -> anyhow::Result<()> {
    let raw = fs::read_to_string(bundle_path)?;
    let bundle: EvidenceBundle = serde_json::from_str(&raw)?;
    let manifest_raw = fs::read_to_string(manifest_path_for(bundle_path))?;
    let manifest: EvidenceManifest = serde_json::from_str(&manifest_raw)?;

    if manifest.record_hash != bundle.record_hash {
        anyhow::bail!("manifest record_hash does not match bundle");
    }
    if manifest.ledger != bundle.ledger {
        anyhow::bail!("manifest ledger does not match bundle");
    }

    let expected = bundle.section_hashes()?;
    if expected != manifest.section_hashes {
        anyhow::bail!("bundle contents do not match the manifest (tampered?)");
    }

    if let Some(sig) = &manifest.signature {
        let signer = manifest
            .signer
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("manifest has a signature but no signer"))?;
        let payload = manifest.signing_payload()?;
        crate::signer::verify_signature(signer, payload.as_bytes(), sig)?;
    }

    if let Some(client) = rpc {
        let latest = client.latest_ledger()?;
        if latest < bundle.ledger {
            anyhow::bail!(
                "pinned ledger {} is ahead of RPC latest ledger {}",
                bundle.ledger,
                latest
            );
        }
    }

    Ok(())
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

    fn sample_bundle() -> EvidenceBundle {
        let r = |ledger: u32, tx: &str| EvidenceRef {
            ledger,
            tx_hash: Some(tx.to_string()),
        };
        build_bundle(
            "aa11",
            42,
            "testnet",
            "Test SDF Network ; September 2015",
            &["CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC".to_string()],
            vec![EvidenceAttestation {
                attester: "GABC".into(),
                record_hash: "aa11".into(),
                reference: r(42, "tx1"),
            }],
            vec![EvidenceAttesterStatus {
                attester: "GABC".into(),
                status: "active".into(),
                reference: r(40, "tx0"),
            }],
            vec![EvidenceDispute {
                dispute_id: "d1".into(),
                record_hash: "aa11".into(),
                reference: r(41, "tx2"),
            }],
            vec![EvidenceRevocation {
                attester: "GDEF".into(),
                record_hash: "aa11".into(),
                reference: r(41, "tx3"),
            }],
            vec![EvidenceStatusListMembership {
                list_id: "list1".into(),
                record_hash: "aa11".into(),
                reference: r(42, "tx4"),
            }],
        )
    }

    #[test]
    fn evidence_export_is_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        export(&sample_bundle(), &a, None).unwrap();
        export(&sample_bundle(), &b, None).unwrap();
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    }

    #[test]
    fn evidence_verify_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.json");
        export(&sample_bundle(), &path, None).unwrap();
        verify_bundle(&path, None).unwrap();

        let mut raw = fs::read_to_string(&path).unwrap();
        raw = raw.replace("tx1", "txX");
        fs::write(&path, raw).unwrap();
        assert!(verify_bundle(&path, None).is_err());
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
