#!/usr/bin/env python3
"""Check that every contract function in the snapshot has a corresponding CLI mapping.

Exits non-zero and prints the unmapped functions if any contract function
listed in scripts/conformance/snapshots/ has no explicit CLI mapping and
is not in the exclusion list.

Usage:
    python3 scripts/conformance/check_cli_coverage.py
"""
import json
import re
import sys
from pathlib import Path

SNAPSHOTS_DIR = Path(__file__).parent / "snapshots"

# The CLI source lives in crates/lafiya-cli/src. After #728 the command bodies
# are split into commands/*.rs modules behind a `Command` trait, so we scan the
# whole src tree (main.rs plus every command module) when verifying that a
# mapped subcommand actually exists.
CLI_SRC_DIR = (
    Path(__file__).resolve().parents[2] / "crates" / "lafiya-cli" / "src"
)

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
}


def load_functions(snapshot_path: Path) -> list[str]:
    data = json.loads(snapshot_path.read_text())
    return [
        entry["name"]
        for entry in data
        if entry.get("kind") == "function_v0"
    ]


def load_cli_source() -> str:
    """Concatenate every Rust source file under the CLI src tree.

    This includes main.rs and all commands/*.rs modules so that subcommands
    defined in the new per-command modules are recognized.
    """
    if not CLI_SRC_DIR.is_dir():
        return ""
    parts: list[str] = []
    for path in sorted(CLI_SRC_DIR.rglob("*.rs")):
        try:
            parts.append(path.read_text())
        except OSError:
            continue
    return "\n".join(parts)


def _subcommand_tokens(cli_path: str) -> list[str]:
    """Split a CLI mapping path into its subcommand tokens.

    Strips flag/value pairs (e.g. "--contract attester-registry") so only the
    positional subcommand words remain.
    """
    tokens: list[str] = []
    skip_next = False
    for raw in cli_path.split():
        if skip_next:
            skip_next = False
            continue
        if raw.startswith("--"):
            # A flag like "--contract" is followed by its value; skip both.
            skip_next = True
            continue
        tokens.append(raw)
    return tokens


def subcommand_defined(cli_path: str, source: str) -> bool:
    """Best-effort check that a mapped subcommand appears in the CLI source.

    We look for the subcommand name as a clap variant / identifier. The final
    token (the leaf subcommand) must appear as an identifier somewhere in the
    source; this is intentionally lenient because clap derives can spell the
    variant in several ways (kebab-case rename, enum variant, etc.).
    """
    tokens = _subcommand_tokens(cli_path)
    if not tokens:
        return True
    leaf = tokens[-1]
    # Match the leaf as a whole identifier, allowing kebab-case to map to
    # snake_case or CamelCase variants used in clap derives.
    candidates = {
        leaf,
        leaf.replace("-", "_"),
        "".join(part.capitalize() for part in leaf.split("-")),
    }
    for candidate in candidates:
        pattern = re.compile(rf"\b{re.escape(candidate)}\b")
        if pattern.search(source):
            return True
    return False


def check() -> bool:
    ok = True
    source = load_cli_source()
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
                continue
            cli_path = CLI_MAPPING[key]
            if source and not subcommand_defined(cli_path, source):
                print(
                    f"MISSING CLI SUBCOMMAND: {contract}::{fn_name} -> "
                    f"'{cli_path}' not found in {CLI_SRC_DIR} "
                    f"(update the command module or CLI_MAPPING)"
                )
                ok = False
    if ok:
        print("OK — every contract function has a CLI mapping or is explicitly excluded.")
    return ok


if __name__ == "__main__":
    sys.exit(0 if check() else 1)
