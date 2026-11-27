#!/usr/bin/env python3
"""Check that every contract function in the snapshot has a corresponding CLI mapping.

Exits non-zero and prints the unmapped functions if any contract function
listed in scripts/conformance/snapshots/ has no explicit CLI mapping and
is not in the exclusion list.

Usage:
    python3 scripts/conformance/check_cli_coverage.py
"""
import json
import sys
from pathlib import Path

SNAPSHOTS_DIR = Path(__file__).parent / "snapshots"

# Functions explicitly excluded from CLI coverage.
# Each entry is (contract_name, function_name) with a short reason.
# Only add entries here for functions that genuinely should not have a CLI
# subcommand (e.g. internal hooks, constructor-only functions).
EXCLUDED: set[tuple[str, str]] = {
    # initialize is called once at deploy time via the deploy subcommand / scripts.
    ("attester-registry", "initialize"),
    ("attestation-registry", "initialize"),
    # add_attesters / remove_attesters are batch variants; the CLI covers the
    # single-item forms (add / remove). Batch ops are intentionally left to the
    # raw stellar CLI to keep the interface simple.
    ("attester-registry", "add_attesters"),
    ("attester-registry", "remove_attesters"),
}

# Map of (contract_name, function_name) -> CLI subcommand path.
# Every public function_v0 entry in the snapshot must appear here or in EXCLUDED.
CLI_MAPPING: dict[tuple[str, str], str] = {
    # ── attester-registry ────────────────────────────────────────────────────
    ("attester-registry", "is_attester"):          "attester is",
    ("attester-registry", "add_attester"):         "attester add",
    ("attester-registry", "add_attester_with_info"): "attester add-with-info",
    ("attester-registry", "update_attester_info"): "attester update-info",
    ("attester-registry", "remove_attester"):      "attester remove",
    ("attester-registry", "suspend_attester"):     "attester suspend",
    ("attester-registry", "reinstate_attester"):   "attester reinstate",
    ("attester-registry", "get_attester_info"):    "attester get-info",
    ("attester-registry", "get_attester_status"):  "attester get-status",
    ("attester-registry", "set_max_attesters"):    "attester set-max-attesters",
    ("attester-registry", "get_max_attesters"):    "attester get-max-attesters",
    ("attester-registry", "get_attester_count"):   "attester get-count",
    ("attester-registry", "get_schema_version"):   "attester get-schema-version",
    ("attester-registry", "upgrade"):              "attester upgrade",
    ("attester-registry", "migrate"):              "attester migrate",
    ("attester-registry", "propose_admin"):        "admin propose --contract attester-registry",
    ("attester-registry", "accept_admin"):         "admin accept --contract attester-registry",
    ("attester-registry", "get_admin"):            "admin get --contract attester-registry",
    ("attester-registry", "pause"):                "ops pause --contract attester-registry",
    ("attester-registry", "unpause"):              "ops unpause --contract attester-registry",
    ("attester-registry", "is_paused"):            "ops is-paused --contract attester-registry",
    # ── attestation-registry ─────────────────────────────────────────────────
    ("attestation-registry", "attest"):                   "attestation attest",
    ("attestation-registry", "revoke_attestation"):       "attestation revoke",
    ("attestation-registry", "get_attestation"):          "attestation get",
    ("attestation-registry", "get_attestation_history"):  "attestation get-history",
    ("attestation-registry", "get_attester_registry"):    "attestation get-attester-registry",
    ("attestation-registry", "set_attester_registry"):    "attestation set-attester-registry",
    ("attestation-registry", "propose_admin"):    "admin propose --contract attestation-registry",
    ("attestation-registry", "accept_admin"):     "admin accept --contract attestation-registry",
    ("attestation-registry", "get_admin"):        "admin get --contract attestation-registry",
    ("attestation-registry", "pause"):            "ops pause --contract attestation-registry",
    ("attestation-registry", "unpause"):          "ops unpause --contract attestation-registry",
    ("attestation-registry", "is_paused"):        "ops is-paused --contract attestation-registry",
    # ── facility-registry ────────────────────────────────────────────────────
    ("facility-registry", "enroll_facility"):          "facility enroll",
    ("facility-registry", "deactivate_facility"):      "facility deactivate",
    ("facility-registry", "reactivate_facility"):      "facility reactivate",
    ("facility-registry", "affiliate_facility"):       "facility affiliate",
    ("facility-registry", "disaffiliate_facility"):    "facility disaffiliate",
    ("facility-registry", "get_facility"):             "facility show",
    ("facility-registry", "get_facility_by_id"):       "facility show --id",
    ("facility-registry", "get_facility_by_portable_id"): "facility show --portable-id",
    ("facility-registry", "resolve_portable_id"):      "facility show --portable-id",
    ("facility-registry", "list_facilities"):          "facility list",
    ("facility-registry", "list_facilities_paginated"): "facility list --cursor",
    ("facility-registry", "list_affiliations"):        "facility list-affiliations",
    ("facility-registry", "list_affiliations_paginated"): "facility list-affiliations --cursor",
    ("facility-registry", "get_facility_count"):       "facility get-count",
    ("facility-registry", "get_schema_version"):       "facility get-schema-version",
    ("facility-registry", "upgrade"):                  "facility upgrade",
    ("facility-registry", "migrate"):                  "facility migrate",
    ("facility-registry", "propose_admin"):    "admin propose --contract facility-registry",
    ("facility-registry", "accept_admin"):     "admin accept --contract facility-registry",
    ("facility-registry", "get_admin"):        "admin get --contract facility-registry",
    ("facility-registry", "pause"):            "ops pause --contract facility-registry",
    ("facility-registry", "unpause"):          "ops unpause --contract facility-registry",
    ("facility-registry", "is_paused"):        "ops is-paused --contract facility-registry",
}


def load_functions(snapshot_path: Path) -> list[str]:
    data = json.loads(snapshot_path.read_text())
    return [
        entry["name"]
        for entry in data
        if entry.get("kind") == "function_v0"
    ]


def check() -> bool:
    ok = True
    for snapshot_file in sorted(SNAPSHOTS_DIR.glob("*.json")):
        contract = snapshot_file.stem  # e.g. "attester-registry"
        functions = load_functions(snapshot_file)
        for fn_name in functions:
            key = (contract, fn_name)
            if key in EXCLUDED:
                continue
            if key not in CLI_MAPPING:
                print(
                    f"MISSING CLI MAPPING: {contract}::{fn_name}  "
                    f"(add to CLI_MAPPING or EXCLUDED in check_cli_coverage.py)"
                )
                ok = False
            else:
                pass  # covered
    if ok:
        print("OK — every contract function has a CLI mapping or is explicitly excluded.")
    return ok


if __name__ == "__main__":
    sys.exit(0 if check() else 1)
