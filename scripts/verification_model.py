#!/usr/bin/env python3
"""Reference implementation of docs/specs/verification-model.md.

    python3 scripts/verification_model.py    # run the truth table, exit 1 on mismatch

Every consumer (contract view, TS SDK, Rust verifier) must produce the same
verdicts as this function for every case in docs/specs/verification-model.json.
"""

import json
import sys
from pathlib import Path
from typing import Any

TABLE = Path(__file__).resolve().parents[1] / "docs" / "specs" / "verification-model.json"


def explain(i: dict[str, Any]) -> tuple[str, str, str | None]:
    """Return (verdict, reason, qualifier) for a fully-populated input dict.

    `reason` names the precedence rule (spec section 3) that matched; the
    differential harness compares it across implementations.
    """
    qualifier = "as_of_bundle" if i["offline"] else None
    if not i["network_matches"]:
        return "WrongNetwork", "network_mismatch", qualifier
    if i["attestation_read"] != "ok":
        return "Indeterminate", "attestation_read_failed", qualifier
    if not i["attestation_found"]:
        return "NotFound", "attestation_missing", qualifier
    if i["revoked"]:
        return "Revoked", "revoked", qualifier
    # Everything below depends on attester state; never guess it.
    if i["attester_status_read"] != "ok":
        return "Indeterminate", "attester_status_read_failed", qualifier
    if i["attested_in_compromise_window"]:
        return "AttesterCompromised", "compromise_window", qualifier
    if i["attester_status"] == "removed":
        return "AttesterRemoved", "attester_removed", qualifier
    if i["age_exceeds_policy"]:
        return "Expired", "age_exceeds_policy", qualifier
    if i["attester_status"] == "suspended":
        return "AttesterSuspended", "attester_suspended", qualifier
    if i["superseded"]:
        return "Superseded", "superseded", qualifier
    return "Verified", "none", qualifier


def evaluate(i: dict[str, Any]) -> tuple[str, str | None]:
    """Return (verdict, qualifier) for a fully-populated input dict."""
    verdict, _, qualifier = explain(i)
    return verdict, qualifier


def main() -> None:
    table = json.loads(TABLE.read_text())
    failures = 0
    for case in table["cases"]:
        got = evaluate({**table["default_input"], **case["input"]})
        want = (case["verdict"], case.get("qualifier"))
        if got[0] not in table["verdicts"] or got != want:
            print(f"[verification-model] {case['id']}: expected {want}, got {got}")
            failures += 1
    total = len(table["cases"])
    print(
        f"[verification-model] {total - failures}/{total} cases pass (model {table['model_version']})"
    )
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
