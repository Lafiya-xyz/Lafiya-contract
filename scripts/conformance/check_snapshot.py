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
import pathlib
import re
import sys
from typing import Any

from contracts import CONTRACTS, SNAPSHOT_DIR
from extract_interface import extract

Key = tuple[str, str]

VERSIONS_FILE = SNAPSHOT_DIR / "interface_versions.json"
_VERSION_RE = re.compile(r"pub const INTERFACE_VERSION: u32 = (\d+);")


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


def load_versions() -> dict[str, int]:
    if not VERSIONS_FILE.exists():
        return {}
    data: dict[str, int] = json.loads(VERSIONS_FILE.read_text())
    return data


def save_versions(versions: dict[str, int]) -> None:
    VERSIONS_FILE.write_text(json.dumps(versions, indent=2, sort_keys=True) + "\n")


def check_one(name: str, cfg: dict[str, pathlib.Path]) -> bool:
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
