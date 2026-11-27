#!/usr/bin/env python3
"""Enforce the pull_request_target policy for GitHub workflows.

`pull_request_target` runs with a write-scoped token and repository secrets, so
it is only allowed in workflows listed in ALLOWLIST below (each entry must be
reviewed by a code owner). Any other workflow that declares the trigger fails
CI, as does an allowlisted workflow that checks out untrusted PR code.

Usage: python3 scripts/ci/check_workflow_policy.py [workflows_dir]
"""

from __future__ import annotations

import sys
from pathlib import Path

import yaml

# Workflows permitted to use `pull_request_target`. Keep this list empty unless
# a workflow has been reviewed for the "pwn request" pattern.
ALLOWLIST: frozenset[str] = frozenset()

TRIGGER = "pull_request_target"


def _triggers(doc: dict) -> set[str]:
    # PyYAML parses the bare key `on` as the boolean True.
    on = doc.get("on", doc.get(True))
    if isinstance(on, str):
        return {on}
    if isinstance(on, list):
        return {str(t) for t in on}
    if isinstance(on, dict):
        return {str(t) for t in on}
    return set()


def _checkout_refs(doc: dict) -> list[str]:
    refs: list[str] = []
    for job in (doc.get("jobs") or {}).values():
        for step in (job or {}).get("steps") or []:
            uses = str((step or {}).get("uses", ""))
            if uses.startswith("actions/checkout"):
                ref = str(((step or {}).get("with") or {}).get("ref", ""))
                refs.append(ref)
    return refs


def check(workflows_dir: Path) -> list[str]:
    errors: list[str] = []
    for path in sorted(workflows_dir.glob("*.y*ml")):
        try:
            doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as exc:  # pragma: no cover - reported by actionlint
            errors.append(f"{path}: unparsable YAML: {exc}")
            continue
        if not isinstance(doc, dict) or TRIGGER not in _triggers(doc):
            continue
        if path.name not in ALLOWLIST:
            errors.append(
                f"{path}: uses `{TRIGGER}` but is not in the reviewed allowlist "
                f"({', '.join(sorted(ALLOWLIST)) or 'empty'})"
            )
            continue
        for ref in _checkout_refs(doc):
            if "github.event.pull_request" in ref or "head" in ref:
                errors.append(
                    f"{path}: allowlisted `{TRIGGER}` workflow checks out "
                    f"untrusted PR code (ref: {ref!r})"
                )
    return errors


def main(argv: list[str]) -> int:
    workflows_dir = Path(argv[1] if len(argv) > 1 else ".github/workflows")
    errors = check(workflows_dir)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print(f"pull_request_target policy OK ({workflows_dir})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
