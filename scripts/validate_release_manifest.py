#!/usr/bin/env python3
"""Validate a release manifest against docs/release-manifest/schema.json and
cross-check internal consistency the JSON Schema alone can't express:

- every contracts[].wasm.sha256 that is non-null must be reproducible: this
  script does not rebuild wasm (that's CI's job), but it does verify that any
  bindings[].generated_from_wasm_sha256 matches its contract's wasm.sha256, so
  a manifest can never claim bindings were generated from a hash the manifest
  itself doesn't also report;
- every sboms[] / vex entry whose file sits next to the manifest (as release
  assets do) must match its recorded sha256, and VEX statement ids must be
  unique;
- a passing release-candidate soak report (manifest `soak` entry) must be
  present next to the manifest, match its recorded sha256, and record
  `status: passed` for the manifest's git_commit (docs/releasing.md#release-candidate-soak).

Usage:
    scripts/validate_release_manifest.py MANIFEST.json
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "docs" / "release-manifest" / "schema.json"

# Contracts that must appear in every release manifest. The incentive-pool is
# held to the same coverage as the registries and multisig.
REQUIRED_CONTRACTS = ("incentive-pool",)


def asset_errors(manifest: dict[str, Any], manifest_dir: Path) -> list[str]:
    errors: list[str] = []
    entries = [(f"sboms[{s['artifact']}]", s) for s in manifest.get("sboms", [])]
    if "vex" in manifest:
        entries.append(("vex", manifest["vex"]))
    for label, entry in entries:
        path = manifest_dir / entry["path"]
        if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            errors.append(f"{label}.sha256 does not match {entry['path']}")
    ids = [s["id"] for s in manifest.get("vex", {}).get("statements", [])]
    for dup in sorted({i for i in ids if ids.count(i) > 1}):
        errors.append(f"vex.statements has more than one statement for {dup}")
    return errors


def soak_errors(manifest: dict[str, Any], manifest_dir: Path) -> list[str]:
    """Require a passing release-candidate soak report (docs/releasing.md#release-candidate-soak).

    The manifest's `soak` entry names the report produced by the RC soak
    workflow. The report must be present next to the manifest, match its
    recorded sha256, and record a passing verdict for the same git commit.
    """
    errors: list[str] = []
    soak = manifest.get("soak")
    if not isinstance(soak, dict):
        errors.append("soak is missing: a passing RC soak report is required before tagging")
        return errors

    if soak.get("status") != "passed":
        errors.append(f"soak.status is {soak.get('status')!r}, expected 'passed'")

    commit = manifest.get("release", {}).get("git_commit")
    if commit and soak.get("git_commit") != commit:
        errors.append(
            f"soak.git_commit ({soak.get('git_commit')}) does not match "
            f"release.git_commit ({commit}) — soak report is for a different build"
        )

    path = manifest_dir / soak["path"]
    if not path.is_file():
        errors.append(f"soak report {soak['path']} not found next to the manifest")
        return errors
    if hashlib.sha256(path.read_bytes()).hexdigest() != soak["sha256"]:
        errors.append(f"soak.sha256 does not match {soak['path']}")
        return errors

    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        errors.append(f"soak report {soak['path']} is not valid JSON: {exc}")
        return errors
    if report.get("status") != "passed":
        errors.append(f"soak report {soak['path']} status is {report.get('status')!r}, expected 'passed'")
    if commit and report.get("git_commit") != commit:
        errors.append(f"soak report {soak['path']} was produced for a different git_commit")
    return errors


def cross_check(manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    contracts = manifest.get("contracts", [])
    wasm_by_contract = {c["name"]: c["wasm"]["sha256"] for c in contracts}

    for name in REQUIRED_CONTRACTS:
        if name not in wasm_by_contract:
            errors.append(f"contracts is missing required artifact '{name}'")

    bound_contracts = set()
    for binding in manifest.get("bindings", []):
        contract = binding["contract"]
        bound_contracts.add(contract)
        expected = wasm_by_contract.get(contract)
        actual = binding["generated_from_wasm_sha256"]
        if expected is not None and actual is not None and expected != actual:
            errors.append(
                f"bindings[{contract}].generated_from_wasm_sha256 ({actual}) does not match "
                f"contracts[{contract}].wasm.sha256 ({expected}) — binding is stale"
            )

    for name in REQUIRED_CONTRACTS:
        if name in wasm_by_contract and name not in bound_contracts:
            errors.append(f"bindings is missing required entry for '{name}'")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="path to the release manifest JSON to validate")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))

    try:
        import jsonschema
    except ImportError:
        print(
            "error: the 'jsonschema' package is required (pip install jsonschema)",
            file=sys.stderr,
        )
        return 2

    validator = jsonschema.Draft202012Validator(schema)
    schema_errors = sorted(validator.iter_errors(manifest), key=lambda e: list(e.path))
    for err in schema_errors:
        path = "/".join(str(p) for p in err.path) or "<root>"
        print(f"schema error at {path}: {err.message}", file=sys.stderr)

    consistency_errors = (
        cross_check(manifest)
        + asset_errors(manifest, args.manifest.parent)
        + soak_errors(manifest, args.manifest.parent)
    )
    for err in consistency_errors:
        print(f"consistency error: {err}", file=sys.stderr)

    if schema_errors or consistency_errors:
        print(
            f"FAIL: {len(schema_errors)} schema error(s), "
            f"{len(consistency_errors)} consistency error(s)",
            file=sys.stderr,
        )
        return 1

    print(f"OK: {args.manifest} is a valid release manifest")
    return 0


if __name__ == "__main__":
    sys.exit(main())
