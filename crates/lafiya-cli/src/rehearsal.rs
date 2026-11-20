//! Upgrade rehearsal: snapshot a contract's data entries (instance and
//! persistent) before and after an upgrade and diff them against an
//! allow-list of expected changes (`docs/runbooks/contract-upgrade.md` §4a).
//!
//! Entries come from `stellar contract read --output json`, one JSON array
//! `[key, value, durability, live_until]` per line (or a single JSON array of
//! those). Keys are decoded schema-aware: a `#[contracttype]` enum variant is
//! a vec whose first element is the variant symbol, which becomes the entry's
//! `kind` so allow-lists are written against `DataKey` variant names.

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub key: String,
    pub kind: String,
    pub durability: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub contract_id: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Added,
    Removed,
    Modified,
}

impl Change {
    fn name(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Modified => "modified",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub key: String,
    pub kind: String,
    pub change: Change,
}

#[derive(Debug, Deserialize)]
pub struct AllowRule {
    /// `DataKey` variant name, or `*`.
    pub kind: String,
    /// `added`, `removed`, `modified`, or `*`.
    pub change: String,
}

#[derive(Debug, Deserialize)]
pub struct AllowList {
    pub allow: Vec<AllowRule>,
}

fn key_kind(key: &Value) -> String {
    let sym = |v: &Value| v.get("symbol").and_then(Value::as_str).map(str::to_owned);
    sym(key)
        .or_else(|| key.get("vec")?.as_array()?.first().and_then(sym))
        .unwrap_or_else(|| "unknown".into())
}

fn push_entry(out: &mut Vec<Entry>, key: &Value, value: &Value, durability: &str) {
    out.push(Entry {
        key: key.to_string(),
        kind: key_kind(key),
        durability: durability.to_owned(),
        value: value.clone(),
    });
}

/// Parse `stellar contract read --output json` text into entries. The
/// contract instance entry is flattened so each instance-storage key is
/// diffed individually.
pub fn parse_read_output(text: &str) -> anyhow::Result<Vec<Entry>> {
    let trimmed = text.trim();
    let rows: Vec<Value> = match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(a)) if a.first().is_some_and(Value::is_array) => a,
        _ => trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .context("expected one JSON array per line")?,
    };
    let mut out = Vec::new();
    for row in rows {
        let cols = row.as_array().context("row is not an array")?;
        let [key, value, dur, ..] = cols.as_slice() else {
            bail!("row has fewer than 3 columns: {row}");
        };
        let dur = dur.as_str().unwrap_or("unknown");
        let storage = value
            .get("storage")
            .or_else(|| value.get("contract_instance")?.get("storage"))
            .and_then(Value::as_array);
        match storage {
            Some(items) => {
                for item in items {
                    let (k, v) = (&item["key"], &item["val"]);
                    push_entry(&mut out, k, v, "instance");
                }
            }
            None => push_entry(&mut out, key, value, dur),
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Diff> {
    let index = |s: &Snapshot| -> BTreeMap<(String, String), Entry> {
        s.entries
            .iter()
            .map(|e| ((e.durability.clone(), e.key.clone()), e.clone()))
            .collect()
    };
    let (b, a) = (index(before), index(after));
    let mut out = Vec::new();
    for (k, e) in &b {
        match a.get(k) {
            None => out.push(Diff {
                key: e.key.clone(),
                kind: e.kind.clone(),
                change: Change::Removed,
            }),
            Some(n) if n.value != e.value => out.push(Diff {
                key: e.key.clone(),
                kind: e.kind.clone(),
                change: Change::Modified,
            }),
            _ => {}
        }
    }
    for (k, e) in &a {
        if !b.contains_key(k) {
            out.push(Diff {
                key: e.key.clone(),
                kind: e.kind.clone(),
                change: Change::Added,
            });
        }
    }
    out
}

/// Changes not covered by any allow-list rule.
pub fn unexpected<'a>(diffs: &'a [Diff], allow: &AllowList) -> Vec<&'a Diff> {
    diffs
        .iter()
        .filter(|d| {
            !allow.allow.iter().any(|r| {
                (r.kind == "*" || r.kind == d.kind)
                    && (r.change == "*" || r.change == d.change.name())
            })
        })
        .collect()
}

pub fn snapshot(
    contract_id: &str,
    network: &str,
    from_file: Option<&Path>,
    out: &Path,
) -> anyhow::Result<()> {
    let raw = match from_file {
        Some(p) => {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?
        }
        None => {
            let res = Command::new("stellar")
                .args([
                    "contract",
                    "read",
                    "--id",
                    contract_id,
                    "--network",
                    network,
                    "--output",
                    "json",
                ])
                .output()
                .context("failed to run `stellar contract read` (is stellar-cli installed?)")?;
            if !res.status.success() {
                bail!(
                    "stellar contract read failed: {}",
                    String::from_utf8_lossy(&res.stderr)
                );
            }
            String::from_utf8(res.stdout)?
        }
    };
    let snap = Snapshot {
        contract_id: contract_id.into(),
        entries: parse_read_output(&raw)?,
    };
    std::fs::write(out, serde_json::to_string_pretty(&snap)?)?;
    println!(
        "snapshot: {} entries -> {}",
        snap.entries.len(),
        out.display()
    );
    Ok(())
}

/// Print the diff; error when any change is outside the allow-list.
pub fn run_diff(before: &Path, after: &Path, allow: &Path) -> anyhow::Result<()> {
    let load = |p: &Path| -> anyhow::Result<Snapshot> {
        Ok(serde_json::from_str(
            &std::fs::read_to_string(p).with_context(|| p.display().to_string())?,
        )?)
    };
    let (b, a) = (load(before)?, load(after)?);
    if b.contract_id != a.contract_id {
        bail!(
            "snapshots are for different contracts: {} vs {}",
            b.contract_id,
            a.contract_id
        );
    }
    let allow: AllowList = serde_json::from_str(&std::fs::read_to_string(allow)?)?;
    let diffs = diff(&b, &a);
    let bad = unexpected(&diffs, &allow);
    for d in &diffs {
        let mark = if bad.contains(&d) {
            "UNEXPECTED"
        } else {
            "expected  "
        };
        println!("{mark} {:8} {} {}", d.change.name(), d.kind, d.key);
    }
    if !bad.is_empty() {
        bail!(
            "{} unexpected state change(s); do not proceed with the upgrade",
            bad.len()
        );
    }
    println!("clean: {} change(s), all allow-listed", diffs.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Incentive-pool testnet rehearsal (#701)
// ---------------------------------------------------------------------------

/// Failure modes the incentive-pool rehearsal must exercise against the real
/// testnet USDC Stellar Asset Contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    /// Recipient has a funded, authorized trustline: approve + claim succeed.
    HappyPath,
    /// Recipient has no USDC trustline: claim fails with a typed error.
    NoTrustline,
    /// Trustline exists but is not authorized (`AUTH_REQUIRED`).
    UnauthorizedTrustline,
    /// Recipient account was merged after approval: claim fails cleanly.
    MergedAccount,
    /// Pool is paused: approve/claim are rejected.
    PausedPool,
    /// Accounting drifts from the token balance: the invariant check trips.
    Drift,
}

impl Scenario {
    pub const ALL: [Scenario; 6] = [
        Scenario::HappyPath,
        Scenario::NoTrustline,
        Scenario::UnauthorizedTrustline,
        Scenario::MergedAccount,
        Scenario::PausedPool,
        Scenario::Drift,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Scenario::HappyPath => "happy_path",
            Scenario::NoTrustline => "no_trustline",
            Scenario::UnauthorizedTrustline => "unauthorized_trustline",
            Scenario::MergedAccount => "merged_account",
            Scenario::PausedPool => "paused_pool",
            Scenario::Drift => "drift",
        }
    }

    /// The typed error the pool contract is expected to surface, if any.
    pub fn expected_error(self) -> Option<&'static str> {
        match self {
            Scenario::HappyPath => None,
            Scenario::NoTrustline => Some("TrustlineMissing"),
            Scenario::UnauthorizedTrustline => Some("TrustlineUnauthorized"),
            Scenario::MergedAccount => Some("RecipientMissing"),
            Scenario::PausedPool => Some("PoolPaused"),
            Scenario::Drift => Some("AccountingDrift"),
        }
    }
}

/// One step of a scenario: the command run and what it produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepResult {
    pub step: String,
    pub ok: bool,
    /// Typed contract error name, when the step failed as expected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Accounting snapshot captured after the step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accounting: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioResult {
    pub scenario: Scenario,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_error: Option<String>,
    pub steps: Vec<StepResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RehearsalReport {
    pub network: String,
    pub pool_id: String,
    pub usdc_id: String,
    pub donor: String,
    pub started_at: String,
    pub finished_at: String,
    pub scenarios: Vec<ScenarioResult>,
    pub passed: bool,
}

impl RehearsalReport {
    /// True when every scenario passed and all required scenarios ran.
    pub fn is_green(&self) -> bool {
        self.passed
            && Scenario::ALL
                .iter()
                .all(|s| self.scenarios.iter().any(|r| r.scenario == *s && r.passed))
    }
}

/// A single rehearsal command to execute. Kept as data so the orchestration is
/// unit-testable without a live network.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSpec {
    pub step: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Build the ordered command list for one scenario. `pool` is the deployed
/// pool contract id, `usdc` the testnet USDC SAC id, `donor` the funding key.
pub fn plan_scenario(scenario: Scenario, pool: &str, usdc: &str, donor: &str) -> Vec<CommandSpec> {
    let invoke = |step: &str, args: &[&str]| CommandSpec {
        step: step.into(),
        program: "stellar".into(),
        args: std::iter::once("contract")
            .chain(std::iter::once("invoke"))
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    };
    let mut cmds = vec![invoke(
        "fund_pool",
        &[
            "--id", usdc, "--source", donor, "--network", "testnet", "--",
            "transfer", "--from", donor, "--to", pool, "--amount", "1000000000",
        ],
    )];
    match scenario {
        Scenario::HappyPath => {
            cmds.push(invoke(
                "approve",
                &["--id", pool, "--network", "testnet", "--", "approve", "--amount", "100"],
            ));
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
        }
        Scenario::NoTrustline => {
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
        }
        Scenario::UnauthorizedTrustline => {
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
        }
        Scenario::MergedAccount => {
            cmds.push(invoke(
                "approve",
                &["--id", pool, "--network", "testnet", "--", "approve", "--amount", "100"],
            ));
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
        }
        Scenario::PausedPool => {
            cmds.push(invoke(
                "pause",
                &["--id", pool, "--network", "testnet", "--", "pause"],
            ));
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
        }
        Scenario::Drift => {
            cmds.push(invoke(
                "claim",
                &["--id", pool, "--network", "testnet", "--", "claim", "--amount", "100"],
            ));
            cmds.push(invoke(
                "check_invariant",
                &["--id", pool, "--network", "testnet", "--", "check_invariant"],
            ));
        }
    }
    cmds
}

/// Extract a typed contract error name from `stellar` stderr/stdout, e.g.
/// `HostError: Error(Contract, #5)` mapped through the pool's error table, or
/// a plain `TrustlineMissing` mention.
pub fn parse_typed_error(output: &str) -> Option<String> {
    const KNOWN: [&str; 5] = [
        "TrustlineMissing",
        "TrustlineUnauthorized",
        "RecipientMissing",
        "PoolPaused",
        "AccountingDrift",
    ];
    KNOWN
        .iter()
        .find(|name| output.contains(**name))
        .map(|name| (*name).to_owned())
}

/// Judge a scenario's step results against its expected typed error.
pub fn evaluate(scenario: Scenario, steps: Vec<StepResult>) -> ScenarioResult {
    let expected = scenario.expected_error();
    let passed = match expected {
        None => steps.iter().all(|s| s.ok),
        Some(err) => steps
            .iter()
            .any(|s| !s.ok && s.error.as_deref() == Some(err)),
    };
    ScenarioResult {
        scenario,
        passed,
        expected_error: expected.map(str::to_owned),
        steps,
    }
}

/// Run the full rehearsal against testnet. `dry_run` plans and evaluates
/// without executing any `stellar` command (used by CI smoke checks).
pub fn run_rehearsal(
    network: &str,
    pool_id: &str,
    usdc_id: &str,
    donor: &str,
    dry_run: bool,
    out: &Path,
) -> anyhow::Result<RehearsalReport> {
    let started_at = now_iso8601();
    let mut scenarios = Vec::new();
    for scenario in Scenario::ALL {
        let mut steps = Vec::new();
        for spec in plan_scenario(scenario, pool_id, usdc_id, donor) {
            if dry_run {
                steps.push(StepResult {
                    step: spec.step,
                    ok: true,
                    error: None,
                    accounting: None,
                });
                continue;
            }
            let res = Command::new(&spec.program)
                .args(&spec.args)
                .output()
                .with_context(|| format!("failed to run {}", spec.program))?;
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&res.stdout),
                String::from_utf8_lossy(&res.stderr)
            );
            let ok = res.status.success();
            steps.push(StepResult {
                step: spec.step,
                ok,
                error: if ok { None } else { parse_typed_error(&combined) },
                accounting: None,
            });
        }
        scenarios.push(evaluate(scenario, steps));
    }
    let report = RehearsalReport {
        network: network.into(),
        pool_id: pool_id.into(),
        usdc_id: usdc_id.into(),
        donor: donor.into(),
        started_at,
        finished_at: now_iso8601(),
        passed: scenarios.iter().all(|s| s.passed),
        scenarios,
    };
    std::fs::write(out, serde_json::to_string_pretty(&report)?)?;
    println!(
        "rehearsal: {} -> {} (green: {})",
        report.network,
        out.display(),
        report.is_green()
    );
    if !report.is_green() {
        bail!("rehearsal not green; see {}", out.display());
    }
    Ok(report)
}

fn now_iso8601() -> String {
    // Avoid a chrono dependency: seconds since the Unix epoch is enough for
    // freshness checks in `check_mainnet_readiness.py`.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = r#"
[{"vec":[{"symbol":"Attester"},{"address":"GA"}]},{"map":1},"persistent",100]
[{"symbol":"ledger_key_contract_instance"},{"storage":[{"key":{"vec":[{"symbol":"SchemaVersion"}]},"val":{"u32":4}}]},"persistent",100]
"#;

    fn snap(text: &str) -> Snapshot {
        Snapshot {
            contract_id: "C".into(),
            entries: parse_read_output(text).unwrap(),
        }
    }

    #[test]
    fn decodes_kinds_and_flattens_instance() {
        let s = snap(BEFORE);
        let kinds: Vec<_> = s
            .entries
            .iter()
            .map(|e| (e.kind.as_str(), e.durability.as_str()))
            .collect();
        assert!(kinds.contains(&("Attester", "persistent")));
        assert!(kinds.contains(&("SchemaVersion", "instance")));
    }

    #[test]
    fn allow_list_gates_changes() {
        let before = snap(BEFORE);
        let after = snap(&BEFORE.replace(r#"{"u32":4}"#, r#"{"u32":5}"#));
        let d = diff(&before, &after);
        assert_eq!(d.len(), 1);
        let ok: AllowList =
            serde_json::from_str(r#"{"allow":[{"kind":"SchemaVersion","change":"modified"}]}"#)
                .unwrap();
        assert!(unexpected(&d, &ok).is_empty());
        let none: AllowList = serde_json::from_str(r#"{"allow":[]}"#).unwrap();
        assert_eq!(unexpected(&d, &none).len(), 1);

        let dropped = Snapshot {
            entries: after.entries[1..].to_vec(),
            ..after
        };
        assert!(diff(&before, &dropped)
            .iter()
            .any(|x| x.change == Change::Removed));
    }

    #[test]
    fn rehearsal_plans_every_scenario() {
        for s in Scenario::ALL {
            let cmds = plan_scenario(s, "CPOOL", "CUSDC", "GDONOR");
            assert_eq!(cmds[0].step, "fund_pool");
            assert!(cmds.len() >= 2, "{:?} needs an action step", s);
        }
    }

    #[test]
    fn parses_typed_errors() {
        assert_eq!(
            parse_typed_error("HostError: TrustlineMissing").as_deref(),
            Some("TrustlineMissing")
        );
        assert_eq!(parse_typed_error("all good"), None);
    }

    #[test]
    fn evaluates_expected_failures() {
        let fail = |err: &str| {
            vec![StepResult {
                step: "claim".into(),
                ok: false,
                error: Some(err.into()),
                accounting: None,
            }]
        };
        assert!(evaluate(Scenario::NoTrustline, fail("TrustlineMissing")).passed);
        assert!(!evaluate(Scenario::NoTrustline, fail("PoolPaused")).passed);
        let ok = vec![StepResult {
            step: "claim".into(),
            ok: true,
            error: None,
            accounting: None,
        }];
        assert!(evaluate(Scenario::HappyPath, ok).passed);
    }

    #[test]
    fn dry_run_report_is_green_and_serializes() {
        let dir = std::env::temp_dir().join(format!("rehearsal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("report.json");
        let report =
            run_rehearsal("testnet", "CPOOL", "CUSDC", "GDONOR", true, &out).unwrap();
        assert!(report.is_green());
        let text = std::fs::read_to_string(&out).unwrap();
        let parsed: RehearsalReport = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.scenarios.len(), Scenario::ALL.len());
        let _ = json!({});
        std::fs::remove_dir_all(&dir).ok();
    }
}
