//! `attestation revoke-by-attester` — revoke all attestations submitted by a
//! specific attester, discovered through on-chain event enumeration.
//!
//! # Issue #400
//!
//! When a CHW turns out to be fraudulent, the admin must revoke everything they
//! attested. The contract only has `revoke_attestation(record_hash)` (one hash
//! per call). This module:
//!
//! 1. **Enumerates** `AttestationRecorded` events for the attestation-registry
//!    via the Soroban RPC `getEvents` method (paged, topic-filtered).
//! 2. **Detects retention-window exhaustion** — Soroban RPC retains events for
//!    ~7 days. If the requested range falls outside the retention window the
//!    command refuses and never reports partial results as complete.
//! 3. **Filters** by the requested attester and time window, deduplicates
//!    `record_hash` values.
//! 4. **Collateral-damage warning** — for each hash, checks whether other
//!    attesters have also attested it. Revoking a hash removes all attestations
//!    for it; the operator is warned about any "bystander" attesters.
//! 5. **Dry-run** — prints the plan without submitting any transactions.
//! 6. **Audit report** — writes a CSV and/or JSON report.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Arguments for `attestation revoke-by-attester`.
#[derive(Debug, Clone)]
pub struct RevokeByAttesterArgs {
    /// Stellar G... address of the attester whose attestations should be revoked.
    pub attester: String,
    /// Earliest ledger sequence to include (inclusive). `None` = from retention start.
    pub since_ledger: Option<u32>,
    /// Latest ledger sequence to include (inclusive). `None` = latest ledger.
    pub until_ledger: Option<u32>,
    /// If `true`, print the plan but do not submit any revocation transactions.
    pub dry_run: bool,
    /// Path to write the CSV audit report. `None` = skip CSV.
    pub audit_csv: Option<PathBuf>,
    /// Path to write the JSON audit report. `None` = skip JSON.
    pub audit_json: Option<PathBuf>,
    /// Network name (for error messages).
    pub network: String,
    /// RPC URL.
    pub rpc_url: String,
    /// Contract ID of the attestation registry.
    pub attestation_registry_id: String,
}

/// A single `AttestationRecorded` event decoded from the RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationEvent {
    /// Hex-encoded 32-byte record hash.
    pub record_hash: String,
    /// Stellar G... attester address.
    pub attester: String,
    /// Ledger sequence number when this event was emitted.
    pub ledger: u32,
    /// Ledger close time (Unix epoch seconds), or 0 if not available.
    pub ledger_close_time: u64,
    /// Transaction hash that emitted this event.
    pub tx_hash: String,
}

/// The current on-chain state of a `record_hash`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttestationState {
    /// The latest attestation is by the target attester.
    LatestByTarget,
    /// The latest attestation is by a different attester; the target attester
    /// is only in the history.
    LatestByOther { latest_attester: String },
    /// All attestations for this hash are already revoked.
    AlreadyRevoked,
    /// State could not be determined (RPC error or not found).
    Unknown,
}

/// Plan entry for a single `record_hash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevocationPlanEntry {
    pub record_hash: String,
    pub state: AttestationState,
    /// Other attesters who would lose their attestations if this hash is revoked.
    pub collateral_attesters: Vec<String>,
    /// Whether the plan includes revoking this hash.
    pub will_revoke: bool,
    /// Result of the revocation, if attempted.
    pub result: RevocationResult,
}

/// Outcome of a single revocation attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RevocationResult {
    /// Not attempted yet (plan phase).
    Pending,
    /// Dry run: would revoke.
    DryRun,
    /// Successfully revoked on-chain.
    Revoked { tx_hash: String },
    /// Skipped (already revoked, or latest attestation is not by the target).
    Skipped { reason: String },
    /// Revocation failed.
    Failed { reason: String },
}

/// Full audit report written to CSV/JSON.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub network: String,
    pub attestation_registry: String,
    pub attester: String,
    pub since_ledger: Option<u32>,
    pub until_ledger: Option<u32>,
    pub dry_run: bool,
    pub events_found: usize,
    pub hashes_found: usize,
    pub revoked: usize,
    pub skipped: usize,
    pub failed: usize,
    pub collateral_damage_hashes: usize,
    pub entries: Vec<RevocationPlanEntry>,
}

/// Errors specific to the revoke-by-attester flow.
#[derive(Debug, thiserror::Error)]
pub enum RevokeError {
    #[error(
        "Event retention window for {network} does not cover the requested range \
         ({since_ledger}–{until_ledger}). RPC retention starts at ledger {rpc_start}.\n\
         \n\
         Remediation: request a range within the RPC retention window \
         (approximately the last 7 days / ~120,960 ledgers), or configure an \
         indexer source with --indexer <url> if one is available.\n\
         Partial results will never be reported as complete."
    )]
    RetentionWindowExceeded {
        network: String,
        since_ledger: u32,
        until_ledger: u32,
        rpc_start: u32,
    },

    #[error("RPC getEvents call failed: {0}")]
    EventsFetchFailed(String),

    #[error("RPC getLatestLedger call failed: {0}")]
    LedgerFetchFailed(String),

    #[error("attestation state check failed for hash {hash}: {reason}")]
    StateFetchFailed { hash: String, reason: String },

    #[error("revocation transaction failed for hash {hash}: {reason}")]
    RevocationFailed { hash: String, reason: String },

    #[error("failed to write audit report to {path}: {reason}")]
    AuditReportFailed { path: String, reason: String },
}

// ---------------------------------------------------------------------------
// RPC event transport (abstracted for testing)
// ---------------------------------------------------------------------------

/// Response from a `getEvents` RPC call.
#[derive(Debug, Deserialize)]
pub struct GetEventsResponse {
    pub events: Vec<RawEvent>,
    #[serde(rename = "latestLedger")]
    pub latest_ledger: u32,
    #[serde(rename = "cursor")]
    pub cursor: Option<String>,
    #[serde(rename = "oldestLedger")]
    pub oldest_ledger: Option<u32>,
}

/// A single event from the RPC.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub ledger: u32,
    #[serde(rename = "ledgerClosedAt")]
    pub ledger_closed_at: Option<String>,
    #[serde(rename = "contractId")]
    pub contract_id: String,
    #[serde(rename = "txHash")]
    pub tx_hash: String,
    pub topic: Vec<String>,
    pub value: Option<serde_json::Value>,
    pub id: Option<String>,
    #[serde(rename = "inSuccessfulContractCall")]
    pub in_successful_contract_call: Option<bool>,
}

/// Abstraction over the RPC transport for event fetching and revocation.
pub trait RevokeTransport: Send + Sync {
    /// Call `getEvents` with the given parameters.
    fn get_events(
        &self,
        rpc_url: &str,
        contract_id: &str,
        start_ledger: u32,
        end_ledger: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<GetEventsResponse, RevokeError>;

    /// Call `getLatestLedger` and return the sequence and oldest available
    /// ledger (retention start).
    fn get_latest_ledger(&self, rpc_url: &str) -> Result<(u32, u32), RevokeError>;

    /// Get the current attestation state for a hash (calls `get_attestation_history`).
    fn get_attestation_history(
        &self,
        rpc_url: &str,
        passphrase: &str,
        attestation_registry_id: &str,
        record_hash: &str,
    ) -> Result<Vec<String>, RevokeError>;

    /// Submit a `revoke_attestation(record_hash)` transaction.
    fn revoke_attestation(
        &self,
        rpc_url: &str,
        passphrase: &str,
        attestation_registry_id: &str,
        record_hash: &str,
        source: &str,
    ) -> Result<String, RevokeError>;
}

// ---------------------------------------------------------------------------
// Main flow
// ---------------------------------------------------------------------------

/// Execute the `attestation revoke-by-attester` flow.
///
/// Returns the complete audit report.
pub fn run_revoke_by_attester(
    args: &RevokeByAttesterArgs,
    passphrase: &str,
    source: &str,
    transport: &dyn RevokeTransport,
) -> Result<AuditReport, RevokeError> {
    // 1. Determine the current ledger and RPC retention window.
    let (latest_ledger, oldest_ledger) = transport.get_latest_ledger(&args.rpc_url)?;

    let since_ledger = args.since_ledger.unwrap_or(oldest_ledger);
    let until_ledger = args.until_ledger.unwrap_or(latest_ledger);

    // 2. Retention check: refuse if the requested range is outside the window.
    if since_ledger < oldest_ledger {
        return Err(RevokeError::RetentionWindowExceeded {
            network: args.network.clone(),
            since_ledger,
            until_ledger,
            rpc_start: oldest_ledger,
        });
    }

    eprintln!(
        "==> Enumerating AttestationRecorded events for attester {} on {} (ledgers {since_ledger}–{until_ledger})",
        args.attester, args.network
    );

    // 3. Enumerate and filter events.
    let events = enumerate_events(
        args,
        since_ledger,
        until_ledger,
        transport,
    )?;

    let filtered: Vec<AttestationEvent> = events
        .into_iter()
        .filter(|e| e.attester.eq_ignore_ascii_case(&args.attester))
        .collect();

    // 4. Deduplicate record hashes.
    let mut seen = HashSet::new();
    let unique_hashes: Vec<String> = filtered
        .iter()
        .filter(|e| seen.insert(e.record_hash.clone()))
        .map(|e| e.record_hash.clone())
        .collect();

    eprintln!(
        "==> Found {} events → {} unique record hashes to examine",
        filtered.len(),
        unique_hashes.len()
    );

    // 5. For each hash: check current state, detect collateral damage.
    let mut plan_entries: Vec<RevocationPlanEntry> = Vec::new();

    for hash in &unique_hashes {
        let entry = build_plan_entry(args, passphrase, hash, transport);
        plan_entries.push(entry);
    }

    // 6. Print plan and collateral-damage warnings.
    print_plan(&plan_entries, &args.attester);

    // 7. Execute (or dry-run).
    let mut revoked_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_count = 0usize;
    let mut collateral_count = 0usize;

    for entry in &mut plan_entries {
        if !entry.will_revoke {
            skipped_count += 1;
            continue;
        }
        if !entry.collateral_attesters.is_empty() {
            collateral_count += 1;
        }
        if args.dry_run {
            entry.result = RevocationResult::DryRun;
            skipped_count += 1;
            continue;
        }
        match transport.revoke_attestation(
            &args.rpc_url,
            passphrase,
            &args.attestation_registry_id,
            &entry.record_hash,
            source,
        ) {
            Ok(tx_hash) => {
                entry.result = RevocationResult::Revoked { tx_hash };
                revoked_count += 1;
            }
            Err(e) => {
                entry.result = RevocationResult::Failed {
                    reason: e.to_string(),
                };
                failed_count += 1;
                eprintln!("ERROR revoking {}: {e}", entry.record_hash);
            }
        }
    }

    // 8. Build and write the audit report.
    let report = AuditReport {
        network: args.network.clone(),
        attestation_registry: args.attestation_registry_id.clone(),
        attester: args.attester.clone(),
        since_ledger: Some(since_ledger),
        until_ledger: Some(until_ledger),
        dry_run: args.dry_run,
        events_found: filtered.len(),
        hashes_found: unique_hashes.len(),
        revoked: revoked_count,
        skipped: skipped_count,
        failed: failed_count,
        collateral_damage_hashes: collateral_count,
        entries: plan_entries,
    };

    write_audit_report(&report, &args.audit_csv, &args.audit_json)?;

    eprintln!(
        "==> Done: {} revoked, {} skipped, {} failed, {} collateral-damage hashes",
        report.revoked, report.skipped, report.failed, report.collateral_damage_hashes
    );

    Ok(report)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn enumerate_events(
    args: &RevokeByAttesterArgs,
    since_ledger: u32,
    until_ledger: u32,
    transport: &dyn RevokeTransport,
) -> Result<Vec<AttestationEvent>, RevokeError> {
    let mut all_events = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let resp = transport.get_events(
            &args.rpc_url,
            &args.attestation_registry_id,
            since_ledger,
            Some(until_ledger),
            cursor.as_deref(),
        )?;

        for raw in &resp.events {
            if let Some(ev) = decode_attestation_event(raw) {
                all_events.push(ev);
            }
        }

        // Page: continue if there's a next cursor and more events.
        if resp.events.is_empty() || resp.cursor.is_none() {
            break;
        }
        cursor = resp.cursor;
    }

    Ok(all_events)
}

fn decode_attestation_event(raw: &RawEvent) -> Option<AttestationEvent> {
    // Topic structure for AttestationRecorded:
    //   topic[0]: event name symbol "AttestationRecorded"
    //   value: { attester: Address, record_hash: BytesN<32>, timestamp: u64 }
    //
    // The exact encoding depends on the Soroban SDK version and the contract's
    // event emission. We look for the event type and extract attester + hash
    // from the value field.
    let topic_name = raw.topic.first()?;
    if !topic_name.contains("AttestationRecorded") {
        return None;
    }

    let value = raw.value.as_ref()?;
    let attester = extract_attester_from_value(value)?;
    let record_hash = extract_record_hash_from_value(value)?;

    let ledger_close_time = raw
        .ledger_closed_at
        .as_deref()
        .and_then(parse_iso8601_to_epoch)
        .unwrap_or(0);

    Some(AttestationEvent {
        record_hash,
        attester,
        ledger: raw.ledger,
        ledger_close_time,
        tx_hash: raw.tx_hash.clone(),
    })
}

fn extract_attester_from_value(value: &serde_json::Value) -> Option<String> {
    // Try common JSON shapes from Soroban RPC event value encoding.
    value
        .get("attester")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            // Some encodings nest the address under a map key.
            value
                .pointer("/map/0/val/address/accountId/publicKeyTypeEd25519")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn extract_record_hash_from_value(value: &serde_json::Value) -> Option<String> {
    value
        .get("record_hash")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .pointer("/bytes")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn parse_iso8601_to_epoch(s: &str) -> Option<u64> {
    // Simple parser for the ISO 8601 format Soroban RPC uses: "2024-01-15T10:30:00Z"
    // Returns Unix epoch seconds.
    // Attempt to parse via a basic manual approach to avoid feature-flag complexity.
    // Format: YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim_end_matches('Z').trim_end_matches('+').to_string();
    let s = format!("{s}Z");
    // Parse as: year(4) - month(2) - day(2) T hour(2) : min(2) : sec(2) Z
    if s.len() < 20 {
        return None;
    }
    let year: u64 = s[0..4].parse().ok()?;
    let month: u64 = s[5..7].parse().ok()?;
    let day: u64 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    // Approximate epoch calculation (ignores leap seconds, close enough for staleness checks).
    let days_from_epoch = days_since_epoch(year, month, day);
    Some(days_from_epoch * 86400 + hour * 3600 + min * 60 + sec)
}

fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    // Days from 1970-01-01. Uses the standard proleptic Gregorian formula.
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 12 } else { month };
    let a = y / 100;
    let b = 2 - a + a / 4;
    let jdn = (365.25 * (y as f64 + 4716.0)) as u64
        + (30.6001 * (m as f64 + 1.0)) as u64
        + day
        + b
        - 1524;
    // Julian Day Number for 1970-01-01 is 2440588.
    jdn.saturating_sub(2_440_588)
}

fn build_plan_entry(
    args: &RevokeByAttesterArgs,
    passphrase: &str,
    hash: &str,
    transport: &dyn RevokeTransport,
) -> RevocationPlanEntry {
    let history = transport.get_attestation_history(
        &args.rpc_url,
        passphrase,
        &args.attestation_registry_id,
        hash,
    );

    let (state, collateral_attesters) = match history {
        Ok(attesters) => {
            let latest = attesters.last().cloned().unwrap_or_default();
            let others: Vec<String> = attesters
                .iter()
                .filter(|a| !a.eq_ignore_ascii_case(&args.attester))
                .cloned()
                .collect();

            let state = if attesters.is_empty() {
                AttestationState::AlreadyRevoked
            } else if latest.eq_ignore_ascii_case(&args.attester) {
                AttestationState::LatestByTarget
            } else {
                AttestationState::LatestByOther {
                    latest_attester: latest,
                }
            };
            (state, others)
        }
        Err(_) => (AttestationState::Unknown, vec![]),
    };

    // Will revoke if the latest attestation involves the target attester
    // (either as the latest or in history), unless already revoked.
    let will_revoke = matches!(
        state,
        AttestationState::LatestByTarget | AttestationState::LatestByOther { .. }
    );

    RevocationPlanEntry {
        record_hash: hash.to_string(),
        state,
        collateral_attesters,
        will_revoke,
        result: RevocationResult::Pending,
    }
}

fn print_plan(entries: &[RevocationPlanEntry], attester: &str) {
    let to_revoke: Vec<_> = entries.iter().filter(|e| e.will_revoke).collect();
    let collateral: Vec<_> = entries
        .iter()
        .filter(|e| !e.collateral_attesters.is_empty())
        .collect();

    println!(
        "\n--- Revocation Plan for attester {} ---",
        attester
    );
    println!(
        "Total hashes: {} | Will revoke: {} | Will skip: {}",
        entries.len(),
        to_revoke.len(),
        entries.len() - to_revoke.len()
    );

    if !collateral.is_empty() {
        println!(
            "\n⚠️  COLLATERAL DAMAGE WARNING: {} hash(es) have co-attesters \
             that will also lose their attestations:",
            collateral.len()
        );
        for e in &collateral {
            println!(
                "  {} — co-attesters: {}",
                &e.record_hash[..12.min(e.record_hash.len())],
                e.collateral_attesters.join(", ")
            );
        }
    }
    println!();
}

fn write_audit_report(
    report: &AuditReport,
    csv_path: &Option<PathBuf>,
    json_path: &Option<PathBuf>,
) -> Result<(), RevokeError> {
    if let Some(path) = json_path {
        let json = serde_json::to_string_pretty(report).map_err(|e| {
            RevokeError::AuditReportFailed {
                path: path.to_string_lossy().to_string(),
                reason: e.to_string(),
            }
        })?;
        std::fs::write(path, json).map_err(|e| RevokeError::AuditReportFailed {
            path: path.to_string_lossy().to_string(),
            reason: e.to_string(),
        })?;
        eprintln!("==> Audit JSON written to {}", path.display());
    }

    if let Some(path) = csv_path {
        let mut csv = String::from(
            "record_hash,state,collateral_attesters,will_revoke,result\n",
        );
        for entry in &report.entries {
            let state_str = match &entry.state {
                AttestationState::LatestByTarget => "LatestByTarget".to_string(),
                AttestationState::LatestByOther { latest_attester } => {
                    format!("LatestByOther({})", latest_attester)
                }
                AttestationState::AlreadyRevoked => "AlreadyRevoked".to_string(),
                AttestationState::Unknown => "Unknown".to_string(),
            };
            let result_str = match &entry.result {
                RevocationResult::Pending => "Pending".to_string(),
                RevocationResult::DryRun => "DryRun".to_string(),
                RevocationResult::Revoked { tx_hash } => format!("Revoked({})", tx_hash),
                RevocationResult::Skipped { reason } => format!("Skipped({})", reason),
                RevocationResult::Failed { reason } => format!("Failed({})", reason),
            };
            csv.push_str(&format!(
                "{},{},{},{},{}\n",
                entry.record_hash,
                state_str,
                entry.collateral_attesters.join(";"),
                entry.will_revoke,
                result_str,
            ));
        }
        std::fs::write(path, csv).map_err(|e| RevokeError::AuditReportFailed {
            path: path.to_string_lossy().to_string(),
            reason: e.to_string(),
        })?;
        eprintln!("==> Audit CSV written to {}", path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Stellar CLI transport (production)
// ---------------------------------------------------------------------------

/// Production transport that shells out to the `stellar` CLI.
pub struct StellarCliTransport;

impl RevokeTransport for StellarCliTransport {
    fn get_events(
        &self,
        rpc_url: &str,
        contract_id: &str,
        start_ledger: u32,
        end_ledger: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<GetEventsResponse, RevokeError> {
        // Build the JSON-RPC getEvents call via curl.
        let end = end_ledger.map_or(serde_json::Value::Null, |e| e.into());
        let cursor_val = cursor.map_or(serde_json::Value::Null, |c| c.into());

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getEvents",
            "params": {
                "startLedger": start_ledger,
                "endLedger": end,
                "filters": [{
                    "type": "contract",
                    "contractIds": [contract_id],
                    "topics": [["*"]]
                }],
                "pagination": {
                    "cursor": cursor_val,
                    "limit": 200
                }
            }
        });

        let body_str = serde_json::to_string(&body)
            .map_err(|e| RevokeError::EventsFetchFailed(e.to_string()))?;

        let output = std::process::Command::new("curl")
            .args([
                "--silent",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body_str,
                rpc_url,
            ])
            .output()
            .map_err(|e| RevokeError::EventsFetchFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(RevokeError::EventsFetchFailed(format!(
                "curl exit {}",
                output.status
            )));
        }

        let resp: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| RevokeError::EventsFetchFailed(e.to_string()))?;

        let result = resp
            .get("result")
            .ok_or_else(|| RevokeError::EventsFetchFailed("missing result field".to_string()))?;

        serde_json::from_value(result.clone())
            .map_err(|e| RevokeError::EventsFetchFailed(e.to_string()))
    }

    fn get_latest_ledger(&self, rpc_url: &str) -> Result<(u32, u32), RevokeError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestLedger",
            "params": {}
        });
        let body_str = serde_json::to_string(&body)
            .map_err(|e| RevokeError::LedgerFetchFailed(e.to_string()))?;
        let output = std::process::Command::new("curl")
            .args([
                "--silent",
                "--max-time",
                "15",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body_str,
                rpc_url,
            ])
            .output()
            .map_err(|e| RevokeError::LedgerFetchFailed(e.to_string()))?;

        let resp: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| RevokeError::LedgerFetchFailed(e.to_string()))?;
        let result = resp
            .get("result")
            .ok_or_else(|| RevokeError::LedgerFetchFailed("missing result".to_string()))?;

        let latest = result
            .get("sequence")
            .and_then(|s| s.as_u64())
            .unwrap_or(0) as u32;
        let oldest = result
            .get("oldestLedger")
            .and_then(|s| s.as_u64())
            .unwrap_or(0) as u32;

        Ok((latest, oldest))
    }

    fn get_attestation_history(
        &self,
        rpc_url: &str,
        passphrase: &str,
        attestation_registry_id: &str,
        record_hash: &str,
    ) -> Result<Vec<String>, RevokeError> {
        // Call `get_attestation_history` via stellar CLI.
        let output = std::process::Command::new("stellar")
            .args([
                "contract",
                "invoke",
                "--id",
                attestation_registry_id,
                "--rpc-url",
                rpc_url,
                "--network-passphrase",
                passphrase,
                "--",
                "get_attestation_history",
                "--record_hash",
                record_hash,
            ])
            .output()
            .map_err(|e| RevokeError::StateFetchFailed {
                hash: record_hash.to_string(),
                reason: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse the returned Vec<Attestation> JSON for attester addresses.
        let attesters = parse_attesters_from_history_output(&stdout);
        Ok(attesters)
    }

    fn revoke_attestation(
        &self,
        rpc_url: &str,
        passphrase: &str,
        attestation_registry_id: &str,
        record_hash: &str,
        source: &str,
    ) -> Result<String, RevokeError> {
        let output = std::process::Command::new("stellar")
            .args([
                "contract",
                "invoke",
                "--id",
                attestation_registry_id,
                "--rpc-url",
                rpc_url,
                "--network-passphrase",
                passphrase,
                "--source",
                source,
                "--send",
                "yes",
                "--",
                "revoke_attestation",
                "--record_hash",
                record_hash,
            ])
            .output()
            .map_err(|e| RevokeError::RevocationFailed {
                hash: record_hash.to_string(),
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RevokeError::RevocationFailed {
                hash: record_hash.to_string(),
                reason: format!("stellar CLI failed: {stderr}"),
            });
        }

        // Extract tx hash from stdout.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let tx_hash = stdout
            .lines()
            .find(|l| l.contains("txHash"))
            .and_then(|l| l.split('"').nth(3))
            .unwrap_or("unknown")
            .to_string();

        Ok(tx_hash)
    }
}

fn parse_attesters_from_history_output(output: &str) -> Vec<String> {
    // The stellar CLI returns JSON; parse attester fields.
    let Ok(v) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return vec![];
    };
    let Some(arr) = v.as_array() else {
        return vec![];
    };
    arr.iter()
        .filter_map(|entry| {
            entry
                .get("attester")
                .and_then(|a| a.as_str())
                .map(str::to_string)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Mock transport for tests
// ---------------------------------------------------------------------------

/// Mock transport for unit and integration tests.
pub struct MockRevokeTransport {
    pub latest_ledger: u32,
    pub oldest_ledger: u32,
    pub events: Vec<RawEvent>,
    /// Map from record_hash to list of attester addresses (history).
    pub attestation_history: HashMap<String, Vec<String>>,
    /// Whether revocations should succeed.
    pub revocation_succeeds: bool,
}

impl MockRevokeTransport {
    pub fn new(
        latest_ledger: u32,
        oldest_ledger: u32,
        events: Vec<RawEvent>,
        history: HashMap<String, Vec<String>>,
        revocation_succeeds: bool,
    ) -> Self {
        Self {
            latest_ledger,
            oldest_ledger,
            events,
            attestation_history: history,
            revocation_succeeds,
        }
    }
}

impl RevokeTransport for MockRevokeTransport {
    fn get_events(
        &self,
        _rpc_url: &str,
        _contract_id: &str,
        _start_ledger: u32,
        _end_ledger: Option<u32>,
        _cursor: Option<&str>,
    ) -> Result<GetEventsResponse, RevokeError> {
        Ok(GetEventsResponse {
            events: self.events.clone(),
            latest_ledger: self.latest_ledger,
            cursor: None,
            oldest_ledger: Some(self.oldest_ledger),
        })
    }

    fn get_latest_ledger(&self, _rpc_url: &str) -> Result<(u32, u32), RevokeError> {
        Ok((self.latest_ledger, self.oldest_ledger))
    }

    fn get_attestation_history(
        &self,
        _rpc_url: &str,
        _passphrase: &str,
        _registry_id: &str,
        record_hash: &str,
    ) -> Result<Vec<String>, RevokeError> {
        Ok(self
            .attestation_history
            .get(record_hash)
            .cloned()
            .unwrap_or_default())
    }

    fn revoke_attestation(
        &self,
        _rpc_url: &str,
        _passphrase: &str,
        _registry_id: &str,
        record_hash: &str,
        _source: &str,
    ) -> Result<String, RevokeError> {
        if self.revocation_succeeds {
            Ok(format!("mock_tx_{}", &record_hash[..8.min(record_hash.len())]))
        } else {
            Err(RevokeError::RevocationFailed {
                hash: record_hash.to_string(),
                reason: "mock failure".to_string(),
            })
        }
    }
}

/// Build a [`RawEvent`] representing an `AttestationRecorded` event.
/// Used in tests.
pub fn make_raw_event(
    attester: &str,
    record_hash: &str,
    ledger: u32,
    contract_id: &str,
) -> RawEvent {
    RawEvent {
        event_type: "contract".to_string(),
        ledger,
        ledger_closed_at: None,
        contract_id: contract_id.to_string(),
        tx_hash: format!("txhash_{ledger}"),
        topic: vec!["AttestationRecorded".to_string()],
        value: Some(serde_json::json!({
            "attester": attester,
            "record_hash": record_hash,
        })),
        id: None,
        in_successful_contract_call: Some(true),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ATTESTER_A: &str = "GABC0000000000000000000000000000000000000000000000000000A";
    const ATTESTER_B: &str = "GABC0000000000000000000000000000000000000000000000000000B";
    const REGISTRY: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const HASH1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const HASH3: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn default_args(dry_run: bool) -> RevokeByAttesterArgs {
        RevokeByAttesterArgs {
            attester: ATTESTER_A.to_string(),
            since_ledger: Some(100),
            until_ledger: Some(200),
            dry_run,
            audit_csv: None,
            audit_json: None,
            network: "testnet".to_string(),
            rpc_url: "https://rpc.example.com".to_string(),
            attestation_registry_id: REGISTRY.to_string(),
        }
    }

    fn build_transport(
        latest: u32,
        oldest: u32,
        events: Vec<RawEvent>,
        history: HashMap<String, Vec<String>>,
        succeeds: bool,
    ) -> MockRevokeTransport {
        MockRevokeTransport::new(latest, oldest, events, history, succeeds)
    }

    #[test]
    fn dry_run_produces_no_revocations() {
        let events = vec![
            make_raw_event(ATTESTER_A, HASH1, 150, REGISTRY),
            make_raw_event(ATTESTER_A, HASH2, 160, REGISTRY),
        ];
        let mut history = HashMap::new();
        history.insert(HASH1.to_string(), vec![ATTESTER_A.to_string()]);
        history.insert(HASH2.to_string(), vec![ATTESTER_A.to_string()]);

        let transport = build_transport(200, 50, events, history, true);
        let report =
            run_revoke_by_attester(&default_args(true), "passphrase", "admin", &transport)
                .unwrap();

        assert_eq!(report.revoked, 0);
        assert_eq!(report.hashes_found, 2);
        assert_eq!(report.dry_run, true);
        for entry in &report.entries {
            assert_eq!(entry.result, RevocationResult::DryRun);
        }
    }

    #[test]
    fn retention_window_exceeded_returns_error() {
        let events = vec![];
        let transport = build_transport(200, 150, events, HashMap::new(), true);
        // Request since=100, but oldest available is 150 — outside window.
        let args = RevokeByAttesterArgs {
            since_ledger: Some(100),
            ..default_args(false)
        };
        let err = run_revoke_by_attester(&args, "passphrase", "admin", &transport).unwrap_err();
        assert!(
            matches!(err, RevokeError::RetentionWindowExceeded { .. }),
            "expected RetentionWindowExceeded, got {err}"
        );
    }

    #[test]
    fn collateral_damage_detected() {
        // HASH3 was attested by both ATTESTER_A (target) and ATTESTER_B (bystander).
        let events = vec![make_raw_event(ATTESTER_A, HASH3, 150, REGISTRY)];
        let mut history = HashMap::new();
        history.insert(
            HASH3.to_string(),
            vec![ATTESTER_A.to_string(), ATTESTER_B.to_string()],
        );

        let transport = build_transport(200, 50, events, history, true);
        let report =
            run_revoke_by_attester(&default_args(true), "passphrase", "admin", &transport)
                .unwrap();

        assert_eq!(report.collateral_damage_hashes, 1);
        let entry = report.entries.iter().find(|e| e.record_hash == HASH3).unwrap();
        assert!(entry.collateral_attesters.contains(&ATTESTER_B.to_string()));
    }

    #[test]
    fn already_revoked_hashes_are_skipped() {
        let events = vec![make_raw_event(ATTESTER_A, HASH1, 150, REGISTRY)];
        let mut history = HashMap::new();
        // Empty history = already revoked.
        history.insert(HASH1.to_string(), vec![]);

        let transport = build_transport(200, 50, events, history, true);
        let report =
            run_revoke_by_attester(&default_args(false), "passphrase", "admin", &transport)
                .unwrap();

        let entry = report.entries.first().unwrap();
        assert_eq!(entry.state, AttestationState::AlreadyRevoked);
        assert!(!entry.will_revoke);
    }

    #[test]
    fn filters_events_by_attester() {
        // ATTESTER_B events should not appear in the plan.
        let events = vec![
            make_raw_event(ATTESTER_A, HASH1, 110, REGISTRY),
            make_raw_event(ATTESTER_B, HASH2, 120, REGISTRY),
            make_raw_event(ATTESTER_A, HASH3, 130, REGISTRY),
        ];
        let mut history = HashMap::new();
        history.insert(HASH1.to_string(), vec![ATTESTER_A.to_string()]);
        history.insert(HASH3.to_string(), vec![ATTESTER_A.to_string()]);

        let transport = build_transport(200, 50, events, history, false);
        let report =
            run_revoke_by_attester(&default_args(true), "passphrase", "admin", &transport)
                .unwrap();

        assert_eq!(report.hashes_found, 2);
        let hashes: Vec<&str> = report.entries.iter().map(|e| e.record_hash.as_str()).collect();
        assert!(hashes.contains(&HASH1));
        assert!(hashes.contains(&HASH3));
        assert!(!hashes.contains(&HASH2));
    }

    #[test]
    fn revocation_failure_is_recorded() {
        let events = vec![make_raw_event(ATTESTER_A, HASH1, 150, REGISTRY)];
        let mut history = HashMap::new();
        history.insert(HASH1.to_string(), vec![ATTESTER_A.to_string()]);

        let transport = build_transport(200, 50, events, history, false /* fails */);
        let report =
            run_revoke_by_attester(&default_args(false), "passphrase", "admin", &transport)
                .unwrap();

        assert_eq!(report.failed, 1);
        let entry = report.entries.first().unwrap();
        assert!(matches!(entry.result, RevocationResult::Failed { .. }));
    }

    #[test]
    fn deduplicates_record_hashes() {
        // Same hash appears twice in events (two attestations from the same attester).
        let events = vec![
            make_raw_event(ATTESTER_A, HASH1, 110, REGISTRY),
            make_raw_event(ATTESTER_A, HASH1, 120, REGISTRY),
        ];
        let mut history = HashMap::new();
        history.insert(HASH1.to_string(), vec![ATTESTER_A.to_string()]);

        let transport = build_transport(200, 50, events, history, true);
        let report =
            run_revoke_by_attester(&default_args(false), "passphrase", "admin", &transport)
                .unwrap();

        assert_eq!(report.hashes_found, 1);
    }
}
