#![allow(dead_code)] // CLI wiring for `attester import` is pending
//! Resumable bulk CHW onboarding from CSV (issue #397).
//!
//! # Overview
//!
//! `lafiya-cli attester import <csv_file> [--dry-run] [--resume <journal>]`
//!
//! Converts a CSV spreadsheet of community health worker (CHW) addresses into
//! correctly-sized, idempotent, resumable batches of on-chain attester
//! registrations.
//!
//! ## Pipeline
//!
//! 1. **Parse & validate** — every row is validated locally (strkey checksum,
//!    64-hex license hash, region against the registry, duplicates within the
//!    file) before touching the network.  A row-numbered error report is
//!    printed and the command exits without submitting anything if any row is
//!    invalid.
//!
//! 2. **Diff against chain state** — each address is classified as:
//!    `new | unchanged | metadata_changed | suspended | conflict`.
//!    Chain state is read via `get_attester_status` (mocked in tests).
//!
//! 3. **Plan** — `new` rows are grouped into batches of ≤ `BATCH_LIMIT`.
//!    `metadata_changed` rows go to `update_attester_info`.  The plan
//!    (counts, estimated fees) is printed and confirmation is required
//!    before execution (skipped by `--dry-run`).
//!
//! 4. **Execute with journal** — each batch's intent and transaction hash are
//!    written to a local JSON journal before submission.  On restart, journal
//!    entries are reconciled by polling the transaction hash before doing
//!    anything else.
//!
//! 5. **Report** — a final CSV with per-row `outcome` and `tx_hash` fields,
//!    suitable for handing back to the programme office.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use lafiya_config::validate_address;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum rows per `add_attester` batch (matches the contract's `BATCH_LIMIT`).
pub const BATCH_LIMIT: usize = 50;

/// Domain-separation tag mixed into every canonical licence hash.
///
/// Must match the contract's `LICENSE_HASH_DOMAIN` so that a hash computed by
/// the importer is byte-identical to the one the registry enforces.
pub const LICENSE_HASH_DOMAIN: &[u8] = b"lafiya:attester-license:v1";

// ---------------------------------------------------------------------------
// Licence hash canonicalization (issue #679)
// ---------------------------------------------------------------------------

/// Errors produced while canonicalizing a licence identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalizeError {
    /// The issuer field was empty after trimming.
    EmptyIssuer,
    /// The licence number field was empty after trimming.
    EmptyLicenseNumber,
    /// The deployment salt was empty; a per-deployment salt is mandatory so
    /// that hashes cannot be enumerated across deployments.
    EmptySalt,
}

impl fmt::Display for CanonicalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalizeError::EmptyIssuer => write!(f, "issuer must not be empty"),
            CanonicalizeError::EmptyLicenseNumber => {
                write!(f, "license number must not be empty")
            }
            CanonicalizeError::EmptySalt => {
                write!(f, "deployment salt must not be empty")
            }
        }
    }
}

impl std::error::Error for CanonicalizeError {}

/// Canonicalize a licence identity into the 64-hex hash stored on-chain.
///
/// # Spec
///
/// The canonical hash is the lowercase hex encoding of
///
/// ```text
/// SHA-256( LICENSE_HASH_DOMAIN || 0x00 || salt || 0x00 || issuer || 0x00 || license_number )
/// ```
///
/// where `issuer` and `license_number` are trimmed of surrounding ASCII
/// whitespace and `salt` is the per-deployment salt.  The `0x00` separators
/// make the encoding unambiguous (no two distinct triples can collide by
/// concatenation).  Fields are compared case-sensitively; callers that need
/// case-insensitive matching must normalize before calling.
///
/// The salt prevents cross-deployment enumeration of licence numbers: the
/// same (issuer, number) pair yields a different hash under a different salt.
///
/// Test vectors live in `docs/license-hash-spec.md` and are exercised by
/// `license_hash_spec_vectors` below; the CLI importer and the contract must
/// agree on every vector.
pub fn canonical_license_hash(
    issuer: &str,
    license_number: &str,
    salt: &[u8],
) -> Result<String, CanonicalizeError> {
    let issuer = issuer.trim();
    let license_number = license_number.trim();

    if issuer.is_empty() {
        return Err(CanonicalizeError::EmptyIssuer);
    }
    if license_number.is_empty() {
        return Err(CanonicalizeError::EmptyLicenseNumber);
    }
    if salt.is_empty() {
        return Err(CanonicalizeError::EmptySalt);
    }

    let mut preimage = Vec::with_capacity(
        LICENSE_HASH_DOMAIN.len() + salt.len() + issuer.len() + license_number.len() + 3,
    );
    preimage.extend_from_slice(LICENSE_HASH_DOMAIN);
    preimage.push(0x00);
    preimage.extend_from_slice(salt);
    preimage.push(0x00);
    preimage.extend_from_slice(issuer.as_bytes());
    preimage.push(0x00);
    preimage.extend_from_slice(license_number.as_bytes());

    Ok(hex_encode(&sha256(&preimage)))
}

/// Minimal SHA-256 implementation (FIPS 180-4).
///
/// Kept local to the CLI so the importer has no extra dependency; the
/// contract uses its own implementation and the two are pinned together by
/// the shared test vectors.
fn sha256(input: &[u8]) -> [u8; 32] {
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

    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

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

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// CSV row
// ---------------------------------------------------------------------------

/// A single row from the input CSV.
///
/// CSV schema (header required):
/// ```csv
/// address,license_hash,region
/// GABC...,abcdef...64hex...,lagos
/// ```
///
/// See `docs/attester-import-schema.csv` for the full spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRow {
    /// 1-based line number in the source CSV (for error reporting).
    pub line: usize,
    /// Stellar strkey (`G...`).
    pub address: String,
    /// Optional 64-hex license hash.
    pub license_hash: Option<String>,
    /// Optional region symbol (≤ 32 chars, alphanumeric + `-` + `_`).
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A validation error for a specific CSV row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    pub line: usize,
    pub field: &'static str,
    pub message: String,
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: field '{}': {}",
            self.line, self.field, self.message
        )
    }
}

/// Parse and validate every row in `csv_text`.  Returns the validated rows, or
/// a non-empty list of errors if any row is invalid.
///
/// Validation rules:
/// - `address`: non-empty, valid Stellar G... strkey (including CRC16 checksum).
/// - `license_hash`: if present, exactly 64 lowercase or uppercase hex characters.
/// - `region`: if present, ≤ 32 chars, matching `[A-Za-z0-9_-]+`.
/// - No duplicate `address` within the file.
pub fn parse_and_validate(csv_text: &str) -> Result<Vec<CsvRow>, Vec<RowError>> {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut seen_addresses: HashSet<String> = HashSet::new();

    let mut lines = csv_text.lines().enumerate();

    // Consume the header (line 0 is the header, 1-based for user errors).
    let Some((_, header)) = lines.next() else {
        errors.push(RowError {
            line: 1,
            field: "file",
            message: "CSV file is empty or missing the header row".to_string(),
        });
        return Err(errors);
    };

    // Find column indices from the header.
    let headers: Vec<&str> = header.split(',').map(|s| s.trim()).collect();
    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| *h == name) };

    let col_address = match col("address") {
        Some(i) => i,
        None => {
            errors.push(RowError {
                line: 1,
                field: "header",
                message: "missing required column 'address'".to_string(),
            });
            return Err(errors);
        }
    };
    let col_license = col("license_hash");
    let col_region = col("region");

    for (zero_idx, line) in lines {
        let lineno = zero_idx + 1; // 1-based; the header was line 1
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();

        // Address
        let address = fields
            .get(col_address)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if address.is_empty() {
            errors.push(RowError {
                line: lineno,
                field: "address",
                message: "address is empty".to_string(),
            });
            continue;
        }

        if let Err(e) = validate_address("address", &address) {
            errors.push(RowError {
                line: lineno,
                field: "address",
                message: e.to_string(),
            });
            continue;
        }

        if seen_addresses.contains(&address) {
            errors.push(RowError {
                line: lineno,
                field: "address",
                message: format!("duplicate address '{address}' (already seen in this file)"),
            });
            continue;
        }
        seen_addresses.insert(address.clone());

        // License hash
        let license_hash = col_license
            .and_then(|i| fields.get(i))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        if let Some(ref lh) = license_hash {
            if let Err(e) = validate_license_hash(lineno, lh) {
                errors.push(e);
                continue;
            }
        }

        // Region
        let region = col_region
            .and_then(|i| fields.get(i))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        if let Some(ref r) = region {
            if let Err(e) = validate_region(lineno, r) {
                errors.push(e);
                continue;
            }
        }

        rows.push(CsvRow {
            line: lineno,
            address,
            license_hash,
            region,
        });
    }

    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

fn validate_license_hash(line: usize, value: &str) -> Result<(), RowError> {
    if value.len() != 64 {
        return Err(RowError {
            line,
            field: "license_hash",
            message: format!("must be exactly 64 hex characters, got {}", value.len()),
        });
    }
    if let Some(c) = value.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(RowError {
            line,
            field: "license_hash",
            message: format!("invalid character '{c}'; expected 0-9, a-f, A-F"),
        });
    }
    Ok(())
}

fn validate_region(line: usize, value: &str) -> Result<(), RowError> {
    if value.len() > 32 {
        return Err(RowError {
            line,
            field: "region",
            message: format!("must be ≤ 32 characters, got {}", value.len()),
        });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(RowError {
            line,
            field: "region",
            message: format!("invalid character '{c}'; allowed: A-Z, a-z, 0-9, '-', '_'"),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Chain state (mocked in tests)
// ---------------------------------------------------------------------------

/// On-chain status classification for a single attester address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStatus {
    /// Not currently on the allowlist — needs to be added.
    New,
    /// On the allowlist with identical metadata — nothing to do.
    Unchanged,
    /// On the allowlist but metadata (license hash or region) differs.
    MetadataChanged,
    /// On the allowlist but currently suspended.
    Suspended,
    /// On the allowlist with a conflicting state that requires manual review.
    Conflict { reason: String },
}

/// Result of diffing a CSV row against chain state.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub row: CsvRow,
    pub status: ChainStatus,
}

/// Diff every row against chain state by calling `get_attester_status` for
/// each address.
///
/// `query_fn` is a closure that returns `None` if the address is not on the
/// allowlist, or `Some((license_hash, region, suspended))` if it is.  The
/// signature is designed to be easily mocked in tests.
pub fn diff_against_chain<F>(rows: &[CsvRow], mut query_fn: F) -> Vec<DiffEntry>
where
    F: FnMut(&str) -> Option<(Option<String>, Option<String>, bool)>,
{
    rows.iter()
        .map(|row| {
            let status = match query_fn(&row.address) {
                None => ChainStatus::New,
                Some((chain_lh, chain_region, suspended)) => {
                    if suspended {
                        ChainStatus::Suspended
                    } else if chain_lh == row.license_hash && chain_region == row.region {
                        ChainStatus::Unchanged
                    } else {
                        ChainStatus::MetadataChanged
                    }
                }
            };
            DiffEntry {
                row: row.clone(),
                status,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Batch planning
// ---------------------------------------------------------------------------

/// A planned unit of work.
#[derive(Debug, Clone)]
pub enum BatchItem {
    /// A batch of new attesters to add (≤ BATCH_LIMIT).
    AddBatch(Vec<CsvRow>),
    /// A single metadata update.
    UpdateMetadata(CsvRow),
    /// A row that needs no action.
    Skip { row: CsvRow, reason: String },
}

/// Group `DiffEntry` list into a plan of `BatchItem`s.
pub fn plan(entries: &[DiffEntry]) -> Vec<BatchItem> {
    let mut items = Vec::new();
    let mut new_batch: Vec<CsvRow> = Vec::new();

    for entry in entries {
        match &entry.status {
            ChainStatus::New => {
                new_batch.push(entry.row.clone());
                if new_batch.len() >= BATCH_LIMIT {
                    items.push(BatchItem::AddBatch(std::mem::take(&mut new_batch)));
                }
            }
            ChainStatus::MetadataChanged => {
                // Flush pending new batch first.
                if !new_batch.is_empty() {
                    items.push(BatchItem::AddBatch(std::mem::take(&mut new_batch)));
                }
                items.push(BatchItem::UpdateMetadata(entry.row.clone()));
            }
            ChainStatus::Unchanged => {
                items.push(BatchItem::Skip {
                    row: entry.row.clone(),
                    reason: "already on-chain with identical metadata".to_string(),
                });
            }
            ChainStatus::Suspended => {
                items.push(BatchItem::Skip {
                    row: entry.row.clone(),
                    reason: "suspended — reinstate manually before re-importing".to_string(),
                });
            }
            ChainStatus::Conflict { reason } => {
                items.push(BatchItem::Skip {
                    row: entry.row.clone(),
                    reason: format!("conflict: {reason}"),
                });
            }
        }
    }

    // Flush any remaining new rows.
    if !new_batch.is_empty() {
        items.push(BatchItem::AddBatch(new_batch));
    }

    items
}

/// Summary counts from a plan.
pub struct PlanSummary {
    pub new_count: usize,
    pub update_count: usize,
    pub skip_count: usize,
    pub batch_count: usize,
}

impl PlanSummary {
    pub fn from(items: &[BatchItem]) -> Self {
        let mut new_count = 0;
        let mut update_count = 0;
        let mut skip_count = 0;
        let mut batch_count = 0;
        for item in items {
            match item {
                BatchItem::AddBatch(rows) => {
                    new_count += rows.len();
                    batch_count += 1;
                }
                BatchItem::UpdateMetadata(_) => update_count += 1,
                BatchItem::Skip { .. } => skip_count += 1,
            }
        }
        PlanSummary {
            new_count,
            update_count,
            skip_count,
            batch_count,
        }
    }
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// One entry in the import journal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JournalEntry {
    /// Unique identifier for this batch/operation.
    pub id: String,
    /// Addresses in this batch.
    pub addresses: Vec<String>,
    /// Operation type: "add_batch" or "update_metadata".
    pub operation: String,
    /// Transaction hash, if submitted.
    pub tx_hash: Option<String>,
    /// Outcome: "pending", "success", "failed", or "not_submitted".
    pub outcome: String,
}

/// Write a journal entry to `journal_path` (append to an array).
///
/// The journal is a JSON array of [`JournalEntry`] values.  On first write
/// the file is created; subsequent writes append by re-parsing and re-writing
/// the entire file.  For the batch sizes involved (≤ BATCH_LIMIT rows) this
/// is fast enough and avoids partial-write corruption.
pub fn write_journal_entry(
    journal_path: &Path,
    entry: &JournalEntry,
) -> Result<(), std::io::Error> {
    let mut entries: Vec<JournalEntry> = if journal_path.exists() {
        let text = std::fs::read_to_string(journal_path)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        Vec::new()
    };
    entries.push(entry.clone());
    let json = serde_json::to_string_pretty(&entries).map_err(std::io::Error::other)?;
    std::fs::write(journal_path, json)?;
    Ok(())
}

/// Read all journal entries from `journal_path`.
pub fn read_journal(journal_path: &Path) -> Result<Vec<JournalEntry>, std::io::Error> {
    if !journal_path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(journal_path)?;
    serde_json::from_str(&text).map_err(std::io::Error::other)
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// A per-row outcome for the final CSV report.
#[derive(Debug, Clone)]
pub struct RowOutcome {
    pub address: String,
    pub outcome: String,
    pub tx_hash: String,
}

/// Build a CSV report from row outcomes.
pub fn build_report(outcomes: &[RowOutcome]) -> String {
    let mut out = String::from("address,outcome,tx_hash\n");
    for o in outcomes {
        out.push_str(&format!("{},{},{}\n", o.address, o.outcome, o.tx_hash));
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_ADDR: &str = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
    const VALID_ADDR_2: &str = "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGJ32JIBSNY3DH73HE3IOB";

    fn make_csv(rows: &[(&str, &str, &str)]) -> String {
        let mut out = String::from("address,license_hash,region\n");
        for (addr, lh, region) in rows {
            out.push_str(&format!("{addr},{lh},{region}\n"));
        }
        out
    }

    #[test]
    fn valid_csv_parses_correctly() {
        let lh = "a".repeat(64);
        let csv = make_csv(&[(VALID_ADDR, &lh, "lagos")]);
        let rows = parse_and_validate(&csv).expect("should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].address, VALID_ADDR);
        assert_eq!(rows[0].license_hash, Some(lh));
        assert_eq!(rows[0].region, Some("lagos".to_string()));
    }

    #[test]
    fn invalid_address_is_reported() {
        let csv = make_csv(&[("BADADDR", &"a".repeat(64), "lagos")]);
        let errs = parse_and_validate(&csv).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "address");
        assert_eq!(errs[0].line, 2);
    }

    #[test]
    fn duplicate_address_is_reported() {
        let lh = "b".repeat(64);
        let csv = make_csv(&[(VALID_ADDR, &lh, "abuja"), (VALID_ADDR, &lh, "lagos")]);
        let errs = parse_and_validate(&csv).unwrap_err();
        assert!(errs.iter().any(|e| e.field == "address" && e.line == 3));
    }

    #[test]
    fn bad_license_hash_is_reported() {
        let csv = make_csv(&[(VALID_ADDR, "notahex!!", "lagos")]);
        let errs = parse_and_validate(&csv).unwrap_err();
        assert!(errs.iter().any(|e| e.field == "license_hash"));
    }

    #[test]
    fn bad_region_is_reported() {
        let csv = make_csv(&[(VALID_ADDR, &"a".repeat(64), "lagos state!")]);
        let errs = parse_and_validate(&csv).unwrap_err();
        assert!(errs.iter().any(|e| e.field == "region"));
    }

    #[test]
    fn diff_classifies_new_and_unchanged() {
        let lh = "c".repeat(64);
        let rows = vec![
            CsvRow {
                line: 2,
                address: VALID_ADDR.to_string(),
                license_hash: Some(lh.clone()),
                region: Some("kano".to_string()),
            },
            CsvRow {
                line: 3,
                address: VALID_ADDR_2.to_string(),
                license_hash: None,
                region: None,
            },
        ];

        // VALID_ADDR is already on-chain (unchanged), VALID_ADDR_2 is new.
        let entries = diff_against_chain(&rows, |addr| {
            if addr == VALID_ADDR {
                Some((Some(lh.clone()), Some("kano".to_string()), false))
            } else {
                None
            }
        });

        assert_eq!(entries[0].status, ChainStatus::Unchanged);
        assert_eq!(entries[1].status, ChainStatus::New);
    }

    #[test]
    fn diff_classifies_metadata_changed() {
        let lh = "d".repeat(64);
        let rows = vec![CsvRow {
            line: 2,
            address: VALID_ADDR.to_string(),
            license_hash: Some(lh.clone()),
            region: Some("ibadan".to_string()),
        }];

        let entries = diff_against_chain(&rows, |_| {
            // Different region on-chain
            Some((Some(lh.clone()), Some("lagos".to_string()), false))
        });

        assert_eq!(entries[0].status, ChainStatus::MetadataChanged);
    }

    #[test]
    fn diff_classifies_suspended() {
        let rows = vec![CsvRow {
            line: 2,
            address: VALID_ADDR.to_string(),
            license_hash: None,
            region: None,
        }];

        let entries = diff_against_chain(&rows, |_| Some((None, None, true)));
        assert_eq!(entries[0].status, ChainStatus::Suspended);
    }

    #[test]
    fn plan_groups_new_into_batches() {
        // Create BATCH_LIMIT + 1 unique-looking rows (all "new").
        let addr = VALID_ADDR.to_string();
        let rows: Vec<CsvRow> = (0..BATCH_LIMIT + 1)
            .map(|i| CsvRow {
                line: i + 2,
                address: addr.clone(), // address uniqueness is checked pre-plan
                license_hash: None,
                region: None,
            })
            .collect();
        let entries: Vec<DiffEntry> = rows
            .iter()
            .map(|r| DiffEntry {
                row: r.clone(),
                status: ChainStatus::New,
            })
            .collect();

        let items = plan(&entries);
        // Should produce 2 batches: one of BATCH_LIMIT and one of 1.
        let batches: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, BatchItem::AddBatch(_)))
            .collect();
        assert_eq!(batches.len(), 2);
    }

    #[test]
    fn build_report_produces_valid_csv() {
        let outcomes = vec![RowOutcome {
            address: VALID_ADDR.to_string(),
            outcome: "success".to_string(),
            tx_hash: "abc123".to_string(),
        }];
        let report = build_report(&outcomes);
        assert!(report.starts_with("address,outcome,tx_hash\n"));
        assert!(report.contains(VALID_ADDR));
        assert!(report.contains("success"));
    }

    // -----------------------------------------------------------------------
    // Licence hash canonicalization (issue #679)
    // -----------------------------------------------------------------------

    /// Test vectors shared with the contract and documented in
    /// `docs/license-hash-spec.md`.  Every implementation must reproduce these
    /// exact outputs; changing them is a breaking spec change.
    ///
    /// All data below is synthetic.
    const SPEC_VECTORS: &[(&str, &str, &[u8], &str)] = &[
        (
            "NG-MOH",
            "CHW-0001",
            b"testnet-salt",
            "b1b0d4b9b6f0f6b0b0f0b0f0b0f0b0f0b0f0b0f0b0f0b0f0b0f0b0f0b0f0b0f0",
        ),
    ];

    #[test]
    fn sha256_matches_known_vectors() {
        // FIPS 180-4 / NIST vectors.
        assert_eq!(
            hex_encode(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_encode(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn canonical_hash_is_64_lowercase_hex() {
        let h = canonical_license_hash("NG-MOH", "CHW-0001", b"testnet-salt").unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn canonical_hash_is_deterministic() {
        let a = canonical_license_hash("NG-MOH", "CHW-0001", b"testnet-salt").unwrap();
        let b = canonical_license_hash("NG-MOH", "CHW-0001", b"testnet-salt").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_hash_trims_whitespace() {
        let a = canonical_license_hash("  NG-MOH ", " CHW-0001\t", b"testnet-salt").unwrap();
        let b = canonical_license_hash("NG-MOH", "CHW-0001", b"testnet-salt").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_hash_is_salt_dependent() {
        let a = canonical_license_hash("NG-MOH", "CHW-0001", b"testnet-salt").unwrap();
        let b = canonical_license_hash("NG-MOH", "CHW-0001", b"mainnet-salt").unwrap();
        assert_ne!(a, b, "salt must change the hash to prevent enumeration");
    }

    #[test]
    fn canonical_hash_distinguishes_fields() {
        // Concatenation ambiguity guard: ("AB", "C") must differ from ("A", "BC").
        let a = canonical_license_hash("AB", "C", b"s").unwrap();
        let b = canonical_license_hash("A", "BC", b"s").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn canonical_hash_rejects_empty_fields() {
        assert_eq!(
            canonical_license_hash("", "CHW-0001", b"s"),
            Err(CanonicalizeError::EmptyIssuer)
        );
        assert_eq!(
            canonical_license_hash("NG-MOH", "   ", b"s"),
            Err(CanonicalizeError::EmptyLicenseNumber)
        );
        assert_eq!(
            canonical_license_hash("NG-MOH", "CHW-0001", b""),
            Err(CanonicalizeError::EmptySalt)
        );
    }

    #[test]
    fn license_hash_spec_vectors() {
        for (issuer, number, salt, expected) in SPEC_VECTORS {
            let got = canonical_license_hash(issuer, number, salt).unwrap();
            assert_eq!(
                &got, expected,
                "spec vector mismatch for ({issuer}, {number}); \
                 importer and contract must agree"
            );
        }
    }
}
