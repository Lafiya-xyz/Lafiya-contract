#!/usr/bin/env python3
"""Reference implementation of docs/specs/verification-model.md.

    python3 scripts/verification_model.py    # run the truth table, exit 1 on mismatch

Every consumer (contract view, TS SDK, Rust verifier) must produce the same
verdicts as this function for every case in docs/specs/verification-model.json.
"""
import json
import sys
from pathlib import Path

TABLE = Path(__file__).resolve().parents[1] / "docs" / "specs" / "verification-model.json"


def evaluate(i):
    """Return (verdict, qualifier) for a fully-populated input dict."""
    qualifier = "as_of_bundle" if i["offline"] else None
    if not i["network_matches"]:
        return "WrongNetwork", qualifier
    if i["attestation_read"] != "ok":
        return "Indeterminate", qualifier
    if not i["attestation_found"]:
        return "NotFound", qualifier
    if i["revoked"]:
        return "Revoked", qualifier
    # Everything below depends on attester state; never guess it.
    if i["attester_status_read"] != "ok":
        return "Indeterminate", qualifier
    if i["attested_in_compromise_window"]:
        return "AttesterCompromised", qualifier
    if i["attester_status"] == "removed":
        return "AttesterRemoved", qualifier
    if i["age_exceeds_policy"]:
        return "Expired", qualifier
    if i["attester_status"] == "suspended":
        return "AttesterSuspended", qualifier
    if i["superseded"]:
        return "Superseded", qualifier
    return "Verified", qualifier


def main():
    table = json.loads(TABLE.read_text())
    failures = 0
    for case in table["cases"]:
        got = evaluate({**table["default_input"], **case["input"]})
        want = (case["verdict"], case.get("qualifier"))
        if got[0] not in table["verdicts"] or got != want:
            print(f"[verification-model] {case['id']}: expected {want}, got {got}")
            failures += 1
    total = len(table["cases"])
    print(f"[verification-model] {total - failures}/{total} cases pass (model {table['model_version']})")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
