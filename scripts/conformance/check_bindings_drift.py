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

from contracts import BINDINGS_CONTRACTS as CONTRACTS

# Bindings packages that live outside the registry/multisig `bindings/`
# tree but must be held to the same drift gate (issue #512).
EXTRA_CONTRACTS: dict[str, dict[str, Path]] = {
    "incentive-pool": {
        "wasm_path": Path("target/wasm32v1-none/release/incentive_pool.wasm"),
        "bindings_dir": Path("bindings/incentive-pool"),
    },
    "attester-registry": {
        "wasm_path": Path("target/wasm32v1-none/release/attester_registry.wasm"),
        "bindings_dir": Path("bindings/attester-registry"),
    },
}

ALL_CONTRACTS: dict[str, dict[str, Path]] = {**CONTRACTS, **EXTRA_CONTRACTS}

# Functions/errors introduced by the bond-accounting (#677), probation
# promotion (#678), bounded-validation (#681) and vesting (#694) work. These
# must be present in the committed bindings for the affected contracts; if a
# contract exposes them but the committed bindings do not, the drift gate
# fails even when the rest of the surface happens to match.
REQUIRED_SURFACE: dict[str, dict[str, set]] = {
    "attester-registry": {
        "functions": {
            "bond_reconcile",
            "check_invariants",
            "try_promote",
        },
        "errors": {
            "InputTooLong",
            "InvalidCharset",
            "InvalidRegionCode",
            "ProbationCriteriaNotMet",
            "BondInvariantViolated",
        },
    },
    "incentive-pool": {
        "functions": {
            "claim_vested",
            "forfeit",
        },
        "errors": {
            "VestingNotClaimable",
            "VestingAlreadyForfeited",
            "InputTooLong",
            "InvalidCharset",
        },
    },
}

FUNCTION_RE = re.compile(r"^\s{2}(\w+):\s*\(.*?options\?: MethodOptions\)", re.MULTILINE)
ERROR_RE = re.compile(r"^\s*(\d+):\s*\{message:\"(\w+)\"\}", re.MULTILINE)


def client_surface(index_ts: str) -> tuple[set[str], dict[int, str]]:
    functions = set(FUNCTION_RE.findall(index_ts))
    errors = {int(code): name for code, name in ERROR_RE.findall(index_ts)}
    return functions, errors


def regenerate(cfg: dict[str, Path], out_dir: Path) -> None:
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


def check_one(name: str, cfg: dict[str, Path], scratch_root: Path) -> bool:
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
        c for c in set(fresh_errs) & set(committed_errs) if fresh_errs[c] != committed_errs[c]
    )

    # Required-surface gate: functions/errors the contract must expose and
    # that the committed bindings must therefore also expose.
    required = REQUIRED_SURFACE.get(name, {})
    required_fns = required.get("functions", set())
    required_err_names = required.get("errors", set())
    committed_err_names = set(committed_errs.values())
    fresh_err_names = set(fresh_errs.values())

    absent_from_contract_fns = sorted(required_fns - fresh_fns)
    absent_from_bindings_fns = sorted(required_fns - committed_fns)
    absent_from_contract_errs = sorted(required_err_names - fresh_err_names)
    absent_from_bindings_errs = sorted(required_err_names - committed_err_names)

    ok = not (
        missing_fns
        or extra_fns
        or missing_errs
        or extra_errs
        or changed_errs
        or absent_from_contract_fns
        or absent_from_bindings_fns
        or absent_from_contract_errs
        or absent_from_bindings_errs
    )

    if ok:
        required_note = ""
        if required_fns or required_err_names:
            required_note = (
                f"; {len(required_fns)} required function(s) and "
                f"{len(required_err_names)} required error(s) present"
            )
        print(
            f"[{name}] OK -- {len(fresh_fns)} function(s), "
            f"{len(fresh_errs)} error code(s) match committed bindings{required_note}"
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
    for fn in absent_from_contract_fns:
        print(f"  ! required function `{fn}` is missing from the contract itself")
    for fn in absent_from_bindings_fns:
        print(f"  ! required function `{fn}` is missing from the committed bindings")
    for err in absent_from_contract_errs:
        print(f"  ! required error `{err}` is missing from the contract itself")
    for err in absent_from_bindings_errs:
        print(f"  ! required error `{err}` is missing from the committed bindings")

    fresh_lines = fresh_ts.count("\n")
    committed_lines = committed_text.count("\n")
    print(
        f"  (informational: regenerated index.ts is {fresh_lines} lines, "
        f"committed is {committed_lines} lines)"
    )
    return False


def main() -> None:
    names = sys.argv[1:] or list(ALL_CONTRACTS)
    unknown = [n for n in names if n not in ALL_CONTRACTS]
    if unknown:
        sys.exit(f"error: unknown contract(s): {', '.join(unknown)}")

    with tempfile.TemporaryDirectory(prefix="lafiya-bindings-check-") as tmp:
        scratch_root = Path(tmp)
        ok = all(check_one(name, ALL_CONTRACTS[name], scratch_root) for name in names)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
