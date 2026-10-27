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
}
