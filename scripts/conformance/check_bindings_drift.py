#!/usr/bin/env python3
"""Detect drift between a contract's built Wasm and its committed
TypeScript client bindings (the `bindings/<contract>` directories that
`lafiya-web` consumes).

    python3 scripts/conformance/check_bindings_drift.py [contract ...]

Regenerates each contract's bindings into a scratch directory with the same
`stellar contract bindings typescript` command `make bindings` uses, then
compares the *client surface* (exported function names and the `Errors`
map) of the regenerated `src/index.ts` against the committed one. This
catches the common failure mode where a contract gains/renames/removes a
function or error variant but nobody remembered to re-run `make bindings`
and commit the result -- the exact drift this repo currently has no CI
enforcement for (see issue #126).

A full-text diff of generated vs. committed `index.ts`/`package.json` is
also reported as informational context (line counts only) since it also
picks up doc-comment/formatting/SDK-dependency-version churn tied to the
`stellar` CLI version rather than the contract interface itself.
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from contracts import CONTRACTS

FUNCTION_RE = re.compile(r"^\s{2}(\w+):\s*\(.*?options\?: MethodOptions\)", re.MULTILINE)
ERROR_RE = re.compile(r"^\s*(\d+):\s*\{message:\"(\w+)\"\}", re.MULTILINE)


def client_surface(index_ts):
    functions = set(FUNCTION_RE.findall(index_ts))
    errors = dict(
        (int(code), name) for code, name in ERROR_RE.findall(index_ts)
    )
    return functions, errors


def regenerate(cfg, out_dir):
    proc = subprocess.run(
        [
            "stellar",
            "contract",
            "bindings",
            "typescript",
            "--wasm",
            str(cfg["wasm_path"]),
            "--output-dir",
            str(out_dir),
            "--overwrite",
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.exit(f"error: bindings generation failed:\n{proc.stderr}")


def check_one(name, cfg, scratch_root):
    committed_ts = cfg["bindings_dir"] / "src" / "index.ts"
    if not committed_ts.exists():
        print(f"[{name}] no committed bindings at {committed_ts}")
        return False

    out_dir = scratch_root / name
    regenerate(cfg, out_dir)
    fresh_ts = (out_dir / "src" / "index.ts").read_text()
    committed_text = committed_ts.read_text()

    fresh_fns, fresh_errs = client_surface(fresh_ts)
    committed_fns, committed_errs = client_surface(committed_text)

    missing_fns = sorted(fresh_fns - committed_fns)
    extra_fns = sorted(committed_fns - fresh_fns)
    missing_errs = sorted(set(fresh_errs) - set(committed_errs))
    extra_errs = sorted(set(committed_errs) - set(fresh_errs))
    changed_errs = sorted(
        c
        for c in set(fresh_errs) & set(committed_errs)
        if fresh_errs[c] != committed_errs[c]
    )

    ok = not (missing_fns or extra_fns or missing_errs or extra_errs or changed_errs)

    if ok:
        print(
            f"[{name}] OK -- {len(fresh_fns)} function(s), "
            f"{len(fresh_errs)} error code(s) match committed bindings"
        )
        return True

    print(f"[{name}] BINDINGS DRIFT vs {committed_ts}:")
    for fn in missing_fns:
        print(f"  - contract exposes `{fn}` but bindings do not (stale bindings)")
    for fn in extra_fns:
        print(f"  + bindings expose `{fn}` but the contract no longer does")
    for code in missing_errs:
        print(f"  - error {code} ({fresh_errs[code]}) missing from bindings' Errors map")
    for code in extra_errs:
        print(f"  + error {code} ({committed_errs[code]}) in bindings but not in the contract")
    for code in changed_errs:
        print(
            f"  ~ error {code} is `{fresh_errs[code]}` in the contract but "
            f"`{committed_errs[code]}` in bindings"
        )

    fresh_lines = fresh_ts.count("\n")
    committed_lines = committed_text.count("\n")
    print(
        f"  (informational: regenerated index.ts is {fresh_lines} lines, "
        f"committed is {committed_lines} lines)"
    )
    return False


def main():
    names = sys.argv[1:] or list(CONTRACTS)
    unknown = [n for n in names if n not in CONTRACTS]
    if unknown:
        sys.exit(f"error: unknown contract(s): {', '.join(unknown)}")

    with tempfile.TemporaryDirectory(prefix="lafiya-bindings-check-") as tmp:
        scratch_root = Path(tmp)
        ok = all(check_one(name, CONTRACTS[name], scratch_root) for name in names)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
