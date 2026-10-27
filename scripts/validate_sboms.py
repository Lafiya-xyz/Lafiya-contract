#!/usr/bin/env python3
"""Validate release SBOMs and the VEX document.

- every SBOM and the VEX document must validate against the CycloneDX 1.6
  JSON schema vendored in docs/release-manifest/cyclonedx/;
- every VEX statement must name an advisory id (RUSTSEC-, GHSA- or CVE-),
  carry an analysis state, and, when it claims the artifacts are not
  affected, a justification plus a written reachability detail;
- every VEX `affects` ref must be a component purl that appears in at least
  one of the SBOMs, so a statement can't silently outlive the dependency
  (except `code_not_present` statements, whose point is that it doesn't);
- with --audit-json (the output of `cargo audit --json`), every advisory
  cargo-audit reports must have a VEX statement.

Usage:
    scripts/validate_sboms.py --vex security/vex/vex.cdx.json [--audit-json A] SBOM...
"""

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_DIR = ROOT / "docs" / "release-manifest" / "cyclonedx"
BOM_SCHEMA = "bom-1.6.schema.json"
ADVISORY_ID = re.compile(
    r"^(RUSTSEC-\d{4}-\d{4}|GHSA(-[23456789cfghjmpqrvwx]{4}){3}|CVE-\d{4}-\d{4,})$"
)
NOT_AFFECTED_STATES = {"not_affected", "false_positive"}


def schema_validator() -> Any:
    import jsonschema
    from referencing import Registry, Resource

    resources = []
    for path in sorted(SCHEMA_DIR.glob("*.schema.json")):
        contents = json.loads(path.read_text(encoding="utf-8"))
        resource = Resource.from_contents(contents)
        resources.append((contents["$id"], resource))
        # bom-1.6 refers to its siblings by relative name.
        resources.append((path.name, resource))
    registry: Registry[Any] = Registry().with_resources(resources)
    schema = json.loads((SCHEMA_DIR / BOM_SCHEMA).read_text(encoding="utf-8"))
    return jsonschema.Draft7Validator(schema, registry=registry)


def schema_errors(validator: Any, doc: Json, label: str) -> list[str]:
    return [
        f"{label}: schema error at {'/'.join(str(p) for p in err.path) or '<root>'}: {err.message}"
        for err in sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
    ]


def sbom_purls(sbom: Json) -> set[str]:
    purls = {c["purl"] for c in sbom.get("components", []) if "purl" in c}
    main = sbom.get("metadata", {}).get("component", {})
    if "purl" in main:
        purls.add(main["purl"])
    return purls


def vex_errors(vex: Json, known_purls: set[str] | None) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    for i, vuln in enumerate(vex.get("vulnerabilities", [])):
        vid = vuln.get("id", "")
        label = f"vex vulnerabilities[{i}] ({vid or 'no id'})"
        if not ADVISORY_ID.match(vid):
            errors.append(f"{label}: id must be a RUSTSEC-, GHSA- or CVE- advisory id")
        if vid in seen:
            errors.append(f"{label}: duplicate statement")
        seen.add(vid)
        analysis = vuln.get("analysis", {})
        state = analysis.get("state")
        if state is None:
            errors.append(f"{label}: analysis.state is required")
        if state in NOT_AFFECTED_STATES:
            if not analysis.get("justification"):
                errors.append(f"{label}: '{state}' needs analysis.justification")
            if len(analysis.get("detail", "").strip()) < 20:
                errors.append(f"{label}: '{state}' needs a reachability analysis.detail")
        affects = vuln.get("affects", [])
        if not affects:
            errors.append(f"{label}: affects must list the affected component purls")
        if known_purls is not None and analysis.get("justification") != "code_not_present":
            for ref in (a.get("ref", "") for a in affects):
                if ref not in known_purls:
                    errors.append(f"{label}: affects ref {ref!r} is not in any SBOM")
    return errors


def audit_errors(audit: Json, vex: Json) -> list[str]:
    covered = {v.get("id") for v in vex.get("vulnerabilities", [])}
    reported: list[Json] = [
        *audit.get("vulnerabilities", {}).get("list", []),
        *(w for ws in audit.get("warnings", {}).values() for w in ws),
    ]
    missing = sorted(
        {
            r["advisory"]["id"]
            for r in reported
            if r.get("advisory") and r["advisory"]["id"] not in covered
        }
    )
    return [
        f"cargo audit reports {a} but the VEX document has no statement for it" for a in missing
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vex", type=Path, required=True)
    parser.add_argument("--audit-json", type=Path, help="output of `cargo audit --json`")
    parser.add_argument("sboms", type=Path, nargs="*")
    args = parser.parse_args()

    try:
        validator = schema_validator()
    except ImportError:
        print("error: the 'jsonschema' package is required", file=sys.stderr)
        return 2

    errors: list[str] = []
    known_purls: set[str] | None = set() if args.sboms else None
    for path in args.sboms:
        sbom = json.loads(path.read_text(encoding="utf-8"))
        errors.extend(schema_errors(validator, sbom, path.name))
        assert known_purls is not None
        known_purls |= sbom_purls(sbom)

    vex = json.loads(args.vex.read_text(encoding="utf-8"))
    errors.extend(schema_errors(validator, vex, args.vex.name))
    errors.extend(vex_errors(vex, known_purls))
    if args.audit_json:
        errors.extend(audit_errors(json.loads(args.audit_json.read_text(encoding="utf-8")), vex))

    for err in errors:
        print(err, file=sys.stderr)
    if errors:
        print(f"FAIL: {len(errors)} error(s)", file=sys.stderr)
        return 1
    print(f"OK: {len(args.sboms)} SBOM(s) and {args.vex} are valid CycloneDX {BOM_SCHEMA[4:7]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
