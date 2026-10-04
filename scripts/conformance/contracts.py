"""Registry of contracts covered by the interface conformance tooling.

Add an entry here when a new Soroban contract should be covered by
`make conformance` (WASM interface snapshots, error/event doc sync, and
binding drift checks).
"""

import pathlib

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WASM_DIR = REPO_ROOT / "target" / "wasm32v1-none" / "release"

CONTRACTS: dict[str, dict[str, pathlib.Path]] = {
    "attester-registry": {
        "crate_dir": REPO_ROOT / "contracts" / "attester-registry",
        "wasm_path": WASM_DIR / "attester_registry.wasm",
        "bindings_dir": REPO_ROOT / "bindings" / "attester-registry",
    },
    "attestation-registry": {
        "crate_dir": REPO_ROOT / "contracts" / "attestation-registry",
        "wasm_path": WASM_DIR / "attestation_registry.wasm",
        "bindings_dir": REPO_ROOT / "bindings" / "attestation-registry",
    },
}

# Contracts whose committed TypeScript bindings are covered by
# `check_bindings_drift.py`. A superset of CONTRACTS: multisig-account has
# client bindings (for browser signer/admin flows) but is not yet covered by
# the interface snapshot or error/event doc checks.
BINDINGS_CONTRACTS = {
    **CONTRACTS,
    "multisig-account": {
        "crate_dir": REPO_ROOT / "contracts" / "multisig-account",
        "wasm_path": WASM_DIR / "multisig_account.wasm",
        "bindings_dir": REPO_ROOT / "bindings" / "multisig-account",
    },
}

SNAPSHOT_DIR = pathlib.Path(__file__).resolve().parent / "snapshots"
