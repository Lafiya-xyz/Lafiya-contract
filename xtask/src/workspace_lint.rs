//! `cargo xtask workspace-lint`: every directory holding a `Cargo.toml` under
//! `crates/`, `contracts/` or `xtask/` must be a workspace member or be listed
//! in `[workspace].exclude`.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const ROOTS: &[&str] = &["crates", "contracts", "xtask"];

pub fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask has no parent directory")?
        .to_path_buf();
    let manifest: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))?.parse()?;
    let ws = manifest
        .get("workspace")
        .and_then(|w| w.as_table())
        .context("root Cargo.toml has no [workspace]")?;
    let listed = |key: &str| -> BTreeSet<String> {
        ws.get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.trim_end_matches('/').to_string()))
            .collect()
    };
    let members = listed("members");
    let excluded = listed("exclude");

    let mut found = Vec::new();
    for dir in ROOTS {
        collect(&root, &root.join(dir), &mut found)?;
    }

    let unlisted: Vec<_> = found
        .iter()
        .filter(|p| !members.contains(*p) && !excluded.contains(*p))
        .collect();
    if !unlisted.is_empty() {
        for p in &unlisted {
            eprintln!(
                "error: `{p}` has a Cargo.toml but is neither a workspace member nor excluded"
            );
        }
        bail!("{} crate(s) missing from [workspace]", unlisted.len());
    }
    println!(
        "workspace-lint: {} crate directories accounted for",
        found.len()
    );
    Ok(())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    if dir.join("Cargo.toml").is_file() {
        let rel = dir.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
        out.push(rel);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir()
            && !matches!(name, "target" | "src" | "tests" | "fuzz" | "node_modules")
            && !name.starts_with('.')
        {
            collect(root, &path, out)?;
        }
    }
    Ok(())
}
