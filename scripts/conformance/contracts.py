"""Registry of contracts covered by the interface conformance tooling.

Add an entry here when a new Soroban contract should be covered by
`make conformance` (WASM interface snapshots, error/event doc sync, and
binding drift checks).
"""

import pathlib

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WASM_DIR = REPO_ROOT / "target" / "wasm32v1-none" / "release"

CONTRACTS = {
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

SNAPSHOT_DIR = pathlib.Path(__file__).resolve().parent / "snapshots"
