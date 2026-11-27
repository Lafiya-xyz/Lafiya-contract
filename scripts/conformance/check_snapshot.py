#!/usr/bin/env python3
"""Diff a contract's built-Wasm interface against its committed snapshot.

    python3 scripts/conformance/check_snapshot.py [--update] [contract ...]

With no contract names, checks every contract in `contracts.py`. Exits
non-zero (and prints an entry-level diff) if any contract's Wasm interface
no longer matches its `snapshots/<contract>.json` file -- i.e. a function
signature, error code, event schema, or shared type changed (or was
removed) since the snapshot was last taken.

The snapshot is also tied to the contract's `INTERFACE_VERSION` constant
(recorded in `snapshots/interface_versions.json`): the check fails if the
constant no longer matches the recorded version, and `--update` refuses a
breaking change (a removed entry, or a changed one other than a new enum or
error variant) without a version bump.

`--update` regenerates the snapshot files instead of checking them; run it
after a deliberate interface change and commit the result.
"""

import json
import os
import pathlib
import re
import sys
from typing import Any

from contracts import CONTRACTS, SNAPSHOT_DIR
from extract_interface import extract

Key = tuple[str, str]

VERSIONS_FILE = SNAPSHOT_DIR / "interface_versions.json"
_VERSION_RE = re.compile(r"pub const INTERFACE_VERSION: u32 = (\d+);")

# File-size budget: warn when any contract source file exceeds this many lines.
# Configurable via the MAX_SOURCE_LINES environment variable.
DEFAULT_MAX_SOURCE_LINES = 800
SOURCE_SUFFIXES = {".rs"}


def snapshot_path(name: str) -> pathlib.Path:
    return SNAPSHOT_DIR / f"{name}.json"


def diff_entries(
    old: list[dict[str, Any]], new: list[dict[str, Any]]
) -> tuple[list[Key], list[Key], list[Key]]:
    old_by_key = {(e["kind"], e["name"]): e["spec"] for e in old}
    new_by_key = {(e["kind"], e["name"]): e["spec"] for e in new}

    removed = sorted(set(old_by_key) - set(new_by_key))
    added = sorted(set(new_by_key) - set(old_by_key))
    changed = sorted(k for k in set(old_by_key) & set(new_by_key) if old_by_key[k] != new_by_key[k])
    return removed, added, changed


_ENUM_KINDS = {"udt_error_enum_v0", "udt_enum_v0"}


def is_breaking_change(kind: str, old_spec: Any, new_spec: Any) -> bool:
    """Whether changing one entry from `old_spec` to `new_spec` breaks callers.

    Adding a variant to an error enum or a valued enum is additive, provided
    every existing variant keeps its name and value. Any other change (a
    signature, return type, struct field, union case, or reordered/renumbered
    variant) is breaking.
    """
    if kind in _ENUM_KINDS:
        old_cases = {(c["name"], c["value"]) for c in old_spec["cases"]}
        new_cases = {(c["name"], c["value"]) for c in new_spec["cases"]}
        return not old_cases <= new_cases
    return True


def is_breaking(old: list[dict[str, Any]], new: list[dict[str, Any]]) -> bool:
    """A removed entry or a breaking change to an existing one breaks callers."""
    removed, _, changed = diff_entries(old, new)
    old_by_key = {(e["kind"], e["name"]): e["spec"] for e in old}
    new_by_key = {(e["kind"], e["name"]): e["spec"] for e in new}
    return bool(removed) or any(
        is_breaking_change(k[0], old_by_key[k], new_by_key[k]) for k in changed
    )


def code_interface_version(crate_dir: pathlib.Path) -> int:
    """Read `INTERFACE_VERSION` from the contract crate's `src/lib.rs`."""
    match = _VERSION_RE.search((crate_dir / "src" / "lib.rs").read_text())
    if match is None:
        raise ValueError(f"no `pub const INTERFACE_VERSION: u32` in {crate_dir}")
    return int(match.group(1))


def max_source_lines() -> int:
    """The configured per-file line budget (env `MAX_SOURCE_LINES`)."""
    raw = os.environ.get("MAX_SOURCE_LINES")
    if raw is None or raw.strip() == "":
        return DEFAULT_MAX_SOURCE_LINES
    try:
        value = int(raw)
    except ValueError:
        raise ValueError(f"MAX_SOURCE_LINES must be an integer, got {raw!r}")
    if value <= 0:
        raise ValueError(f"MAX_SOURCE_LINES must be positive, got {value}")
    return value


def source_files(crate_dir: pathlib.Path) -> list[pathlib.Path]:
    """All contract source files under `<crate_dir>/src`."""
    src = crate_dir / "src"
    if not src.is_dir():
        return []
    return sorted(
        p for p in src.rglob("*") if p.is_file() and p.suffix in SOURCE_SUFFIXES
    )


def check_file_sizes(crate_dir: pathlib.Path, budget: int) -> bool:
    """Warn (non-fatally) about source files over `budget` lines.

    Returns True so callers can chain it with other checks without failing the
    run; the warning is informational and does not affect the exit status.
    """
    for path in source_files(crate_dir):
        lines = len(path.read_text().splitlines())
        if lines > budget:
            print(
                f"warning: {path} has {lines} lines "
                f"(budget {budget}; set MAX_SOURCE_LINES to adjust)"
            )
    return True


def load_versions() -> dict[str, int]:
    if not VERSIONS_FILE.exists():
        return {}
    data: dict[str, int] = json.loads(VERSIONS_FILE.read_text())
    return data


def save_versions(versions: dict[str, int]) -> None:
    VERSIONS_FILE.write_text(json.dumps(versions, indent=2, sort_keys=True) + "\n")


def check_one(name: str, cfg: dict[str, pathlib.Path]) -> bool:
    budget = max_source_lines()
    check_file_sizes(cfg["crate_dir"], budget)

    snap_file = snapshot_path(name)
    current = extract(cfg["wasm_path"])

    if not snap_file.exists():
        print(f"[{name}] no snapshot yet at {snap_file} -- run with --update")
        return False

    ok = True
    recorded = load_versions().get(name)
    version = code_interface_version(cfg["crate_dir"])
    if recorded != version:
        print(
            f"[{name}] INTERFACE_VERSION is {version} but the snapshot records "
            f"{recorded} -- run with --update"
        )
        ok = False

    baseline = json.loads(snap_file.read_text())
    removed, added, changed = diff_entries(baseline, current)

    if not (removed or added or changed):
        print(f"[{name}] OK -- interface matches {snap_file.name}")
        return ok

    print(f"[{name}] INTERFACE DRIFT vs {snap_file.name}:")
    for kind, entry_name in removed:
        print(f"  - removed {kind}: {entry_name}")
    for kind, entry_name in added:
        print(f"  + added   {kind}: {entry_name}")
    for kind, entry_name in changed:
        print(f"  ~ changed {kind}: {entry_name}")
    if is_breaking(baseline, current):
        print(f"  breaking change: bump INTERFACE_VERSION (currently {version})")
    return False


def update_one(name: str, cfg: dict[str, pathlib.Path]) -> bool:
    budget = max_source_lines()
    check_file_sizes(cfg["crate_dir"], budget)

    snap_file = snapshot_path(name)
    current = extract(cfg["wasm_path"])
    versions = load_versions()
    version = code_interface_version(cfg["crate_dir"])
    recorded = versions.get(name)

    if snap_file.exists() and recorded is not None:
        baseline = json.loads(snap_file.read_text())
        if is_breaking(baseline, current) and version <= recorded:
            print(
                f"[{name}] refusing to update: breaking interface change without "
                f"an INTERFACE_VERSION bump (still {version})"
            )
            return False

    SNAPSHOT_DIR.mkdir(parents=True, exist_ok=True)
    snap_file.write_text(json.dumps(current, indent=2, sort_keys=True) + "\n")
    versions[name] = version
    save_versions(versions)
    print(f"[{name}] wrote {snap_file} (interface v{version})")
    return True


def main(argv: list[str] | None = None) -> None:
    args = sys.argv[1:] if argv is None else argv
    update = "--update" in args
    names = [a for a in args if a != "--update"] or list(CONTRACTS)

    unknown = [n for n in names if n not in CONTRACTS]
    if unknown:
        sys.exit(f"error: unknown contract(s): {', '.join(unknown)}")

    step = update_one if update else check_one
    ok = True
    for name in names:
        ok = step(name, CONTRACTS[name]) and ok
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
