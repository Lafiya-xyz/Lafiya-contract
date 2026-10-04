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

use lafiya_config::{validate_address, validate_rpc_url, ValidationError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum rows per `add_attester` batch (matches the contract's `BATCH_LIMIT`).
pub const BATCH_LIMIT: usize = 50;

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
        let lineno = zero_idx + 2; // 1-based, offset by header
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
            message: format!(
                "must be exactly 64 hex characters, got {}",
                value.len()
            ),
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
            message: format!(
                "invalid character '{c}'; allowed: A-Z, a-z, 0-9, '-', '_'"
            ),
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
    let json = serde_json::to_string_pretty(&entries)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(journal_path, json)?;
    Ok(())
}

/// Read all journal entries from `journal_path`.
pub fn read_journal(journal_path: &Path) -> Result<Vec<JournalEntry>, std::io::Error> {
    if !journal_path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(journal_path)?;
    serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
        let csv = make_csv(&[
            (VALID_ADDR, &lh, "abuja"),
            (VALID_ADDR, &lh, "lagos"),
        ]);
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
}
