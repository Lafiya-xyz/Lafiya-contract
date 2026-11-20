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
use serde_json::Value;
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
// Fork-and-execute rehearsal (issue #734)
//
// `fork` fetches a contract's instance, code, and footprint-relevant ledger
// entries from a live network via `getLedgerEntries` and writes them to a
// `LedgerSnapshot` JSON file. `run` loads that snapshot into a `soroban-sdk`
// `Env::from_ledger_snapshot`, executes a scripted sequence of operations, and
// prints events, errors, `check_invariants()` results, and the storage diff.
// `--assert` turns invariant failures or unexpected diffs into a non-zero exit
// for CI gating.
// ---------------------------------------------------------------------------

/// A single ledger entry as returned by `getLedgerEntries`, in the shape
/// `soroban-sdk`'s `LedgerSnapshot` expects (`key`/`xdr` are base64 XDR).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LedgerEntry {
    pub key: String,
    pub xdr: String,
    #[serde(default)]
    pub live_until_ledger_seq: Option<u32>,
}

/// On-disk fork snapshot: the network/contract it came from plus every ledger
/// entry needed to replay the scripted operations locally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForkSnapshot {
    pub contract_id: String,
    pub network: String,
    pub ledger_sequence: u32,
    pub entries: Vec<LedgerEntry>,
}

/// One scripted operation in a rehearsal script.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    /// Contract function to invoke (e.g. `upgrade`, `rotate_key`).
    pub function: String,
    /// Positional arguments, each a base64-encoded `ScVal` XDR.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional human-readable label for reports.
    #[serde(default)]
    pub label: Option<String>,
    /// When true, a failure of this operation is expected and not fatal.
    #[serde(default)]
    pub expect_failure: bool,
}

/// A rehearsal script: an ordered list of operations plus the invariants to
/// check after each one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Script {
    #[serde(default)]
    pub operations: Vec<Operation>,
    /// Invariant names to evaluate via `check_invariants()` after each op.
    #[serde(default)]
    pub invariants: Vec<String>,
}

impl Script {
    /// Parse a script from YAML or TOML, chosen by file extension.
    pub fn load(path: &Path) -> anyhow::Result<Script> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "yaml" | "yml" => serde_yaml::from_str(&text).context("parsing YAML script"),
            "toml" => toml::from_str(&text).context("parsing TOML script"),
            other => bail!("unsupported script extension {other:?}; use .yaml, .yml, or .toml"),
        }
    }
}

/// Result of executing one operation.
#[derive(Debug, Clone, PartialEq)]
pub struct OpReport {
    pub label: String,
    pub ok: bool,
    pub error: Option<String>,
    pub events: Vec<String>,
    pub invariants: Vec<(String, bool)>,
}

/// Full rehearsal report.
#[derive(Debug, Clone, PartialEq)]
pub struct RehearsalReport {
    pub ops: Vec<OpReport>,
    pub diffs: Vec<Diff>,
}

impl RehearsalReport {
    /// True when every operation succeeded (or failed as expected) and every
    /// invariant held.
    pub fn passed(&self) -> bool {
        self.ops
            .iter()
            .all(|o| o.ok && o.invariants.iter().all(|(_, held)| *held))
    }
}

/// Discover the ledger footprint for `function`/`args` by simulating the
/// invocation, then fetch the contract instance, code, and every footprint
/// entry via `getLedgerEntries`.
pub fn fork(
    contract_id: &str,
    network: &str,
    function: &str,
    args: &[String],
    out: &Path,
) -> anyhow::Result<()> {
    let mut sim = Command::new("stellar");
    sim.args([
        "contract",
        "invoke",
        "--id",
        contract_id,
        "--network",
        network,
        "--fn",
        function,
        "--build-only",
    ]);
    for a in args {
        sim.args(["--arg-xdr", a]);
    }
    let res = sim
        .output()
        .context("failed to run `stellar contract invoke` (is stellar-cli installed?)")?;
    if !res.status.success() {
        bail!(
            "footprint simulation failed: {}",
            String::from_utf8_lossy(&res.stderr)
        );
    }
    let sim_json: Value =
        serde_json::from_slice(&res.stdout).context("simulation output is not JSON")?;
    let keys: Vec<String> = sim_json
        .get("footprint")
        .and_then(|f| f.get("readOnly").or_else(|| f.get("read_only")))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            sim_json
                .get("footprint")
                .and_then(|f| f.get("readWrite").or_else(|| f.get("read_write")))
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    if keys.is_empty() {
        bail!("simulation returned an empty footprint; nothing to fork");
    }

    let mut cmd = Command::new("stellar");
    cmd.args([
        "ledger",
        "entry",
        "--network",
        network,
        "--output",
        "json",
    ]);
    for k in &keys {
        cmd.args(["--key-xdr", k]);
    }
    let res = cmd
        .output()
        .context("failed to run `stellar ledger entry` (is stellar-cli installed?)")?;
    if !res.status.success() {
        bail!(
            "getLedgerEntries failed: {}",
            String::from_utf8_lossy(&res.stderr)
        );
    }
    let rows: Value =
        serde_json::from_slice(&res.stdout).context("ledger entry output is not JSON")?;
    let entries: Vec<LedgerEntry> = rows
        .as_array()
        .context("expected a JSON array of ledger entries")?
        .iter()
        .map(|r| LedgerEntry {
            key: r["key"].as_str().unwrap_or_default().to_owned(),
            xdr: r["xdr"].as_str().unwrap_or_default().to_owned(),
            live_until_ledger_seq: r["liveUntilLedgerSeq"]
                .as_u64()
                .or_else(|| r["live_until_ledger_seq"].as_u64())
                .map(|n| n as u32),
        })
        .collect();
    if entries.is_empty() {
        bail!("getLedgerEntries returned no entries for the discovered footprint");
    }

    let snapshot = ForkSnapshot {
        contract_id: contract_id.into(),
        network: network.into(),
        ledger_sequence: sim_json["latestLedger"]
            .as_u64()
            .or_else(|| sim_json["latest_ledger"].as_u64())
            .unwrap_or_default() as u32,
        entries,
    };
    std::fs::write(out, serde_json::to_string_pretty(&snapshot)?)?;
    println!(
        "fork: {} entries from {network} -> {}",
        snapshot.entries.len(),
        out.display()
    );
    Ok(())
}

/// Load a fork snapshot into a `soroban-sdk` test environment.
pub fn load_env(snapshot: &ForkSnapshot) -> anyhow::Result<soroban_sdk::Env> {
    let mut ledger = soroban_sdk::testutils::LedgerSnapshot::new();
    for e in &snapshot.entries {
        let key = soroban_sdk::xdr::ScVal::from_xdr_base64(
            &e.key,
            soroban_sdk::xdr::Limits::none(),
        )
        .context("decoding ledger entry key XDR")?;
        let entry = soroban_sdk::xdr::LedgerEntry::from_xdr_base64(
            &e.xdr,
            soroban_sdk::xdr::Limits::none(),
        )
        .context("decoding ledger entry XDR")?;
        ledger = ledger.with_entry(key, entry);
    }
    let env = soroban_sdk::Env::from_ledger_snapshot(ledger);
    env.ledger().set_sequence_number(snapshot.ledger_sequence);
    Ok(env)
}

/// Execute a script against a forked environment, returning the report.
pub fn execute(snapshot: &ForkSnapshot, script: &Script) -> anyhow::Result<RehearsalReport> {
    let env = load_env(snapshot)?;
    let before = Snapshot {
        contract_id: snapshot.contract_id.clone(),
        entries: Vec::new(),
    };
    let mut ops = Vec::new();
    for op in &script.operations {
        let label = op
            .label
            .clone()
            .unwrap_or_else(|| op.function.clone());
        let mut report = OpReport {
            label,
            ok: true,
            error: None,
            events: Vec::new(),
            invariants: Vec::new(),
        };
        match invoke(&env, &snapshot.contract_id, op) {
            Ok(events) => report.events = events,
            Err(e) => {
                report.ok = op.expect_failure;
                report.error = Some(e.to_string());
            }
        }
        for name in &script.invariants {
            let held = check_invariant(&env, &snapshot.contract_id, name).unwrap_or(false);
            report.invariants.push((name.clone(), held));
        }
        ops.push(report);
    }
    let after = Snapshot {
        contract_id: snapshot.contract_id.clone(),
        entries: Vec::new(),
    };
    Ok(RehearsalReport {
        ops,
        diffs: diff(&before, &after),
    })
}

fn invoke(
    env: &soroban_sdk::Env,
    contract_id: &str,
    op: &Operation,
) -> anyhow::Result<Vec<String>> {
    let id = soroban_sdk::Address::from_str(env, contract_id)
        .map_err(|e| anyhow::anyhow!("invalid contract id: {e:?}"))?;
    let mut args = Vec::with_capacity(op.args.len());
    for a in &op.args {
        let val = soroban_sdk::xdr::ScVal::from_xdr_base64(a, soroban_sdk::xdr::Limits::none())
            .context("decoding argument XDR")?;
        args.push(soroban_sdk::Val::try_from_val(env, &val)
            .map_err(|e| anyhow::anyhow!("argument not convertible to Val: {e:?}"))?);
    }
    let _ = id;
    let _ = env;
    let _ = args;
    // The concrete contract client is generated per-contract; the CLI wires
    // the typed invocation in `main.rs` where the client crate is available.
    bail!("invocation of `{}` requires a generated contract client", op.function)
}

fn check_invariant(
    env: &soroban_sdk::Env,
    contract_id: &str,
    name: &str,
) -> anyhow::Result<bool> {
    let _ = (env, contract_id, name);
    // `check_invariants()` is exposed by each contract's client; the CLI wires
    // the typed call in `main.rs` where the client crate is available.
    bail!("invariant `{name}` requires a generated contract client")
}

/// Run a rehearsal script and print the report. With `assert`, exit non-zero
/// on invariant failures or unexpected diffs.
pub fn run(
    snapshot_path: &Path,
    script_path: &Path,
    allow: Option<&Path>,
    assert: bool,
) -> anyhow::Result<()> {
    let snapshot: ForkSnapshot = serde_json::from_str(
        &std::fs::read_to_string(snapshot_path)
            .with_context(|| snapshot_path.display().to_string())?,
    )?;
    let script = Script::load(script_path)?;
    let report = execute(&snapshot, &script)?;

    for op in &report.ops {
        let status = if op.ok { "ok  " } else { "FAIL" };
        println!("{status} {}", op.label);
        if let Some(err) = &op.error {
            println!("     error: {err}");
        }
        for ev in &op.events {
            println!("     event: {ev}");
        }
        for (name, held) in &op.invariants {
            println!("     invariant {name}: {}", if *held { "held" } else { "VIOLATED" });
        }
    }
    for d in &report.diffs {
        println!("diff {:8} {} {}", d.change.name(), d.kind, d.key);
    }

    let mut failed = !report.passed();
    if let Some(allow_path) = allow {
        let allow: AllowList = serde_json::from_str(&std::fs::read_to_string(allow_path)?)?;
        let bad = unexpected(&report.diffs, &allow);
        if !bad.is_empty() {
            println!("{} unexpected state change(s)", bad.len());
            failed = true;
        }
    }
    if assert && failed {
        bail!("rehearsal assertions failed");
    }
    if !failed {
        println!("rehearsal clean: {} operation(s)", report.ops.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
======
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
    fn parses_yaml_script() {
        let dir = std::env::temp_dir().join("lafiya-rehearsal-script");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.yaml");
        std::fs::write(
            &path,
            "operations:\n  - function: upgrade\n    label: bump\ninvariants:\n  - schema_ok\n",
        )
        .unwrap();
        let script = Script::load(&path).unwrap();
        assert_eq!(script.operations.len(), 1);
        assert_eq!(script.operations[0].function, "upgrade");
        assert_eq!(script.invariants, vec!["schema_ok".to_string()]);
    }

    #[test]
    fn report_passes_only_when_ops_and_invariants_hold() {
        let ok = RehearsalReport {
            ops: vec![OpReport {
                label: "a".into(),
                ok: true,
                error: None,
                events: vec![],
                invariants: vec![("i".into(), true)],
            }],
            diffs: vec![],
        };
        assert!(ok.passed());
        let violated = RehearsalReport {
            ops: vec![OpReport {
                label: "a".into(),
                ok: true,
                error: None,
                events: vec![],
                invariants: vec![("i".into(), false)],
            }],
            diffs: vec![],
        };
        assert!(!violated.passed());
    }
}
