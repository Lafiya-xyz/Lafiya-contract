#!/usr/bin/env python3
"""Diff a contract's built-Wasm interface against its committed snapshot.

    python3 scripts/conformance/check_snapshot.py [--update] [contract ...]

With no contract names, checks every contract in `contracts.py`. Exits
non-zero (and prints an entry-level diff) if any contract's Wasm interface
no longer matches its `snapshots/<contract>.json` file -- i.e. a function
signature, error code, event schema, or shared type changed (or was
removed) since the snapshot was last taken.

`--update` regenerates the snapshot files instead of checking them; run it
after a deliberate interface change and commit the result.
"""
import json
import sys

from contracts import CONTRACTS, SNAPSHOT_DIR
from extract_interface import extract


def snapshot_path(name):
    return SNAPSHOT_DIR / f"{name}.json"


def diff_entries(old, new):
    old_by_key = {(e["kind"], e["name"]): e["spec"] for e in old}
    new_by_key = {(e["kind"], e["name"]): e["spec"] for e in new}

    removed = sorted(set(old_by_key) - set(new_by_key))
    added = sorted(set(new_by_key) - set(old_by_key))
    changed = sorted(
        k
        for k in set(old_by_key) & set(new_by_key)
        if old_by_key[k] != new_by_key[k]
    )
    return removed, added, changed


def check_one(name, cfg):
    snap_file = snapshot_path(name)
    current = extract(cfg["wasm_path"])

    if not snap_file.exists():
        print(f"[{name}] no snapshot yet at {snap_file} -- run with --update")
        return False

    baseline = json.loads(snap_file.read_text())
    removed, added, changed = diff_entries(baseline, current)

    if not (removed or added or changed):
        print(f"[{name}] OK -- interface matches {snap_file.name}")
        return True

    print(f"[{name}] INTERFACE DRIFT vs {snap_file.name}:")
    for kind, entry_name in removed:
        print(f"  - removed {kind}: {entry_name}")
    for kind, entry_name in added:
        print(f"  + added   {kind}: {entry_name}")
    for kind, entry_name in changed:
        print(f"  ~ changed {kind}: {entry_name}")
    return False


def update_one(name, cfg):
    snap_file = snapshot_path(name)
    current = extract(cfg["wasm_path"])
    SNAPSHOT_DIR.mkdir(parents=True, exist_ok=True)
    snap_file.write_text(json.dumps(current, indent=2, sort_keys=True) + "\n")
    print(f"[{name}] wrote {snap_file}")


def main():
    args = sys.argv[1:]
    update = "--update" in args
    names = [a for a in args if a != "--update"] or list(CONTRACTS)

    unknown = [n for n in names if n not in CONTRACTS]
    if unknown:
        sys.exit(f"error: unknown contract(s): {', '.join(unknown)}")

    if update:
        for name in names:
            update_one(name, CONTRACTS[name])
        return

    ok = True
    for name in names:
        ok = check_one(name, CONTRACTS[name]) and ok
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
