//! `cargo xtask generate`: regenerate every committed generated artifact
//! (interface snapshots, TypeScript bindings, error/event catalogs, event
//! docs and decoders, the OpenAPI operation index and the release manifest)
//! in one pass. `--check` restores the committed files afterwards and fails
//! with a review-friendly diff if any of them drifted: JSON files are
//! compared semantically (one line per changed key path), everything else
//! line by line.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const BINDINGS: [&str; 3] = [
    "attester-registry",
    "attestation-registry",
    "multisig-account",
];
const SCRATCH: &str = "target/golden";
const OPENAPI_SRC: &str = "crates/lafiya-indexer/openapi.yaml";
const OPENAPI_GOLDEN: &str = "golden/openapi-operations.txt";
const MANIFEST_GOLDEN: &str = "golden/release-manifest.json";
/// Release-manifest fields that change on every build; masked in the golden.
const MANIFEST_VOLATILE: [&str; 4] = ["git_commit", "generated_at", "sha256", "size_bytes"];

/// Generators that rewrite their outputs in place: (name, command, outputs).
/// Order matters: the catalog and event generators read the snapshots.
const IN_PLACE: [(&str, &[&str], &[&str]); 4] = [
    (
        "interface snapshots",
        &[
            "python3",
            "scripts/conformance/check_snapshot.py",
            "--update",
        ],
        &["scripts/conformance/snapshots"],
    ),
    (
        "error/event catalogs",
        &["python3", "scripts/conformance/gen_catalog.py"],
        &[
            "catalog/errors.json",
            "catalog/events.json",
            "bindings/attester-registry/src/catalog.ts",
            "bindings/attestation-registry/src/catalog.ts",
            "bindings/multisig-account/src/catalog.ts",
        ],
    ),
    (
        "events doc",
        &["python3", "scripts/conformance/gen_events_doc.py"],
        &["docs/events.md"],
    ),
    (
        "events decoders",
        &["python3", "scripts/conformance/gen_events_ts.py"],
        &["packages/events/src/generated.ts"],
    ),
];

pub fn run(check: bool) -> Result<()> {
    crate::docs::build_wasm()?;
    let mut tracked: Vec<PathBuf> = Vec::new();
    for (_, _, outputs) in IN_PLACE {
        for out in outputs {
            tracked.extend(files_under(Path::new(out))?);
        }
    }
    for c in BINDINGS {
        tracked.push(bindings_index(c));
    }
    tracked.push(OPENAPI_GOLDEN.into());
    tracked.push(MANIFEST_GOLDEN.into());
    let before = read_all(&tracked);

    let result = regenerate();
    // New snapshot files may appear on regeneration; pick them up too.
    for (_, _, outputs) in IN_PLACE {
        for out in outputs {
            for f in files_under(Path::new(out))? {
                if !tracked.contains(&f) {
                    tracked.push(f);
                }
            }
        }
    }
    let after = read_all(&tracked);
    if check {
        restore(&before)?;
    }
    result?;

    let mut drifted = 0;
    for path in &tracked {
        let (old, new) = (before.get(path), after.get(path));
        if old == new {
            continue;
        }
        drifted += 1;
        if check {
            println!(
                "--- {} (committed)\n+++ {} (regenerated)",
                path.display(),
                path.display()
            );
            let (old, new) = (
                old.map_or("", String::as_str),
                new.map_or("", String::as_str),
            );
            print!("{}", render_diff(path, old, new));
        } else {
            println!("wrote {}", path.display());
        }
    }
    if check && drifted > 0 {
        bail!("{drifted} generated artifact(s) out of date; run `cargo xtask generate` and commit");
    }
    println!(
        "generated artifacts: {} tracked, {drifted} changed",
        tracked.len()
    );
    Ok(())
}

fn regenerate() -> Result<()> {
    for c in BINDINGS {
        let wasm = format!("target/wasm32v1-none/release/{}.wasm", c.replace('-', "_"));
        let out = Path::new(SCRATCH).join("bindings").join(c);
        let _ = std::fs::remove_dir_all(&out);
        let mut cmd = Command::new("stellar");
        cmd.args([
            "contract",
            "bindings",
            "typescript",
            "--wasm",
            &wasm,
            "--overwrite",
        ]);
        cmd.arg("--output-dir").arg(&out);
        exec("bindings", &mut cmd)?;
        std::fs::copy(out.join("src/index.ts"), bindings_index(c))?;
    }
    for (name, argv, _) in IN_PLACE {
        exec(name, Command::new(argv[0]).args(&argv[1..]))?;
    }
    let spec =
        std::fs::read_to_string(OPENAPI_SRC).with_context(|| format!("reading {OPENAPI_SRC}"))?;
    write(OPENAPI_GOLDEN, &openapi_operations(&spec))?;
    let out = Command::new("python3")
        .args(["scripts/generate_release_manifest.py", "--pretty"])
        .output()?;
    if !out.status.success() {
        bail!(
            "release manifest generator failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut manifest: Value = serde_json::from_slice(&out.stdout)?;
    mask(&mut manifest);
    write(
        MANIFEST_GOLDEN,
        &(serde_json::to_string_pretty(&manifest)? + "\n"),
    )
}

fn exec(name: &str, cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("running the {name} generator"))?;
    if !status.success() {
        bail!("the {name} generator failed");
    }
    Ok(())
}

fn bindings_index(contract: &str) -> PathBuf {
    Path::new("bindings").join(contract).join("src/index.ts")
}

fn write(path: &str, body: &str) -> Result<()> {
    std::fs::create_dir_all(Path::new(path).parent().expect("has parent"))?;
    Ok(std::fs::write(path, body)?)
}

fn files_under(path: &Path) -> Result<Vec<PathBuf>> {
    if !path.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path)? {
        files.extend(files_under(&entry?.path())?);
    }
    files.sort();
    Ok(files)
}

fn read_all(paths: &[PathBuf]) -> BTreeMap<PathBuf, String> {
    paths
        .iter()
        .filter_map(|p| Some((p.clone(), std::fs::read_to_string(p).ok()?)))
        .collect()
}

fn restore(before: &BTreeMap<PathBuf, String>) -> Result<()> {
    for (path, body) in before {
        std::fs::write(path, body)?;
    }
    Ok(())
}

/// `METHOD /path operationId` lines from the indexer's OpenAPI document, so a
/// change to the served API surface shows up as a one-line golden diff.
fn openapi_operations(spec: &str) -> String {
    let (mut ops, mut path, mut in_paths) = (Vec::<(String, Option<&str>)>::new(), "", false);
    for line in spec.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        match (indent, trimmed.strip_suffix(':')) {
            (0, Some(k)) => in_paths = k == "paths",
            (2, Some(k)) if in_paths => path = k,
            (4, Some(k)) if in_paths => ops.push((format!("{} {path}", k.to_uppercase()), None)),
            _ => {
                if let (true, Some(id)) = (in_paths, trimmed.strip_prefix("operationId:")) {
                    if let Some(last) = ops.last_mut() {
                        last.1 = Some(id.trim());
                    }
                }
            }
        }
    }
    ops.iter()
        .map(|(op, id)| id.map_or(op.clone(), |id| format!("{op} {id}")) + "\n")
        .collect()
}

fn mask(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if MANIFEST_VOLATILE.contains(&k.as_str()) && !child.is_null() {
                    *child = Value::String("<masked>".into());
                } else {
                    mask(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask),
        _ => {}
    }
}

fn render_diff(path: &Path, old: &str, new: &str) -> String {
    if path.extension().is_some_and(|e| e == "json") {
        if let (Ok(a), Ok(b)) = (serde_json::from_str(old), serde_json::from_str(new)) {
            let mut out = String::new();
            json_diff("$", &a, &b, &mut out);
            if !out.is_empty() {
                return out;
            }
        }
    }
    line_diff(old, new)
}

/// One line per differing key path: `~` changed, `-` removed, `+` added.
fn json_diff(at: &str, a: &Value, b: &Value, out: &mut String) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                match y.get(k) {
                    Some(vb) => json_diff(&format!("{at}.{k}"), va, vb, out),
                    None => out.push_str(&format!("- {at}.{k}: {va}\n")),
                }
            }
            for (k, vb) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                out.push_str(&format!("+ {at}.{k}: {vb}\n"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            for i in 0..x.len().max(y.len()) {
                match (x.get(i), y.get(i)) {
                    (Some(va), Some(vb)) => json_diff(&format!("{at}[{i}]"), va, vb, out),
                    (Some(va), None) => out.push_str(&format!("- {at}[{i}]: {va}\n")),
                    (None, Some(vb)) => out.push_str(&format!("+ {at}[{i}]: {vb}\n")),
                    (None, None) => unreachable!(),
                }
            }
        }
        _ if a != b => out.push_str(&format!("~ {at}: {a} -> {b}\n")),
        _ => {}
    }
}

/// Minimal LCS line diff showing only changed lines with their line numbers.
fn line_diff(old: &str, new: &str) -> String {
    let (a, b): (Vec<_>, Vec<_>) = (old.lines().collect(), new.lines().collect());
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            (i, j) = (i + 1, j + 1);
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] > lcs[i + 1][j]) {
            out.push_str(&format!("+{:>5} | {}\n", j + 1, b[j]));
            j += 1;
        } else {
            out.push_str(&format!("-{:>5} | {}\n", i + 1, a[i]));
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_diff_reports_key_paths() {
        let a = serde_json::json!({"x": 1, "y": [1, 2], "gone": true});
        let b = serde_json::json!({"x": 2, "y": [1], "new": "v"});
        let mut out = String::new();
        json_diff("$", &a, &b, &mut out);
        assert_eq!(
            out,
            "- $.gone: true\n~ $.x: 1 -> 2\n- $.y[1]: 2\n+ $.new: \"v\"\n"
        );
    }

    #[test]
    fn line_diff_shows_only_changes() {
        assert_eq!(
            line_diff("a\nb\nc\n", "a\nB\nc\n"),
            "-    2 | b\n+    2 | B\n"
        );
    }

    #[test]
    fn openapi_operations_lists_methods() {
        let spec = "openapi: 3.0.0\npaths:\n  /health:\n    get:\n      operationId: health\n  /x:\n    post:\n      summary: s\ncomponents:\n  a:\n";
        assert_eq!(openapi_operations(spec), "GET /health health\nPOST /x\n");
    }

    #[test]
    fn mask_hides_volatile_fields() {
        let mut v = serde_json::json!({"release": {"git_commit": "abc", "git_tag": null}});
        mask(&mut v);
        assert_eq!(
            v,
            serde_json::json!({"release": {"git_commit": "<masked>", "git_tag": null}})
        );
    }
}
