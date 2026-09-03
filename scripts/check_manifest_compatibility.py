#!/usr/bin/env python3
"""Check a release manifest against a consumer's declared compatibility requirements.

This is the "consumer compatibility check" a downstream repo (e.g. lafiya-web)
runs in its own CI against a Lafiya-contract release manifest before bumping
its pinned contract/binding version, per
docs/adr/0010-release-manifest-and-compatibility.md.

A requirements file (see docs/release-manifest/examples/lafiya-web.requirements.json)
declares, per contract, the minimum storage schema version and binding
package-version range the consumer's code was written against. This script
fails (non-zero exit) if the manifest doesn't satisfy them, printing exactly
which requirement was violated.

Usage:
    scripts/check_manifest_compatibility.py MANIFEST.json REQUIREMENTS.json
"""
import argparse
import json
import re
import sys
from pathlib import Path


def parse_caret_range(range_str):
    """'^X.Y.Z' SemVer caret range, per npm semver rules: the leftmost
    non-zero component is pinned, so ^1.2.3 allows <2.0.0 but ^0.2.3 allows
    only <0.3.0 and ^0.0.3 allows only <0.0.4 (0.x releases are not assumed
    backward compatible with each other)."""
    m = re.match(r"^\^(\d+)\.(\d+)\.(\d+)$", range_str)
    if not m:
        raise ValueError(f"unsupported version range syntax: {range_str!r} (only ^X.Y.Z supported)")
    major, minor, patch = (int(g) for g in m.groups())
    lo = (major, minor, patch)
    if major > 0:
        hi = (major + 1, 0, 0)
    elif minor > 0:
        hi = (major, minor + 1, 0)
    else:
        hi = (major, minor, patch + 1)
    return lo, hi


def parse_version(version_str):
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)", version_str)
    if not m:
        raise ValueError(f"unsupported version syntax: {version_str!r}")
    return tuple(int(g) for g in m.groups())


def version_in_range(version_str, range_str):
    lo, hi = parse_caret_range(range_str)
    v = parse_version(version_str)
    return lo <= v < hi


def check(manifest, requirements):
    errors = []
    contracts_by_name = {c["name"]: c for c in manifest.get("contracts", [])}
    bindings_by_contract = {b["contract"]: b for b in manifest.get("bindings", [])}

    for contract_name, req in requirements.get("contracts", {}).items():
        contract = contracts_by_name.get(contract_name)
        if contract is None:
            errors.append(f"{contract_name}: manifest has no such contract")
            continue

        min_schema = req.get("min_storage_schema_version")
        if min_schema is not None:
            actual = contract["storage_schema_version"]
            if actual is None or actual < min_schema:
                errors.append(
                    f"{contract_name}: requires storage_schema_version >= {min_schema}, "
                    f"manifest has {actual}"
                )

        binding_range = req.get("binding_version_range")
        if binding_range is not None:
            binding = bindings_by_contract.get(contract_name)
            if binding is None:
                errors.append(f"{contract_name}: requires bindings, manifest has none")
            elif not version_in_range(binding["package_version"], binding_range):
                errors.append(
                    f"{contract_name}: requires bindings version {binding_range}, "
                    f"manifest has {binding['package_version']}"
                )

    for event_req in requirements.get("events", []):
        contract_name, event_name = event_req["contract"], event_req["name"]
        matches = [
            e
            for e in manifest.get("events", [])
            if e["contract"] == contract_name and e["name"] == event_name
        ]
        if not matches:
            errors.append(f"{contract_name}.{event_name}: required event missing from manifest")
        elif matches[0]["compatibility"] == "breaking":
            errors.append(
                f"{contract_name}.{event_name}: manifest reports a breaking change "
                "since the consumer's last checked release"
            )
        elif matches[0]["compatibility"] == "removed":
            errors.append(f"{contract_name}.{event_name}: required event was removed")

    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="path to the release manifest JSON to check")
    parser.add_argument("requirements", type=Path, help="path to the consumer's requirements JSON")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    requirements = json.loads(args.requirements.read_text(encoding="utf-8"))

    errors = check(manifest, requirements)
    if errors:
        repo = requirements.get("repo", "<consumer>")
        print(f"INCOMPATIBLE: {repo} vs {args.manifest}", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(f"COMPATIBLE: {requirements.get('repo', '<consumer>')} vs {args.manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
