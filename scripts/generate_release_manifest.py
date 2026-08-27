#!/usr/bin/env python3
"""Generate a Lafiya release manifest (docs/release-manifest/schema.json) from repo state.

Reads the workspace version, per-contract storage schema version, built wasm
artifacts (if present), generated TypeScript bindings, contract event
definitions, and config/networks.toml, and emits one JSON document that binds
them together for a release. See docs/adr/0010-release-manifest-and-compatibility.md.

Usage:
    scripts/generate_release_manifest.py [--previous PATH] [--pretty] [-o PATH]

Building the wasm artifacts first (`make wasm`) lets the manifest include real
sha256 hashes; without a build, wasm.sha256 is null and the manifest still
generates (useful for CI dry-runs and for validating the tooling itself).
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# name -> (crate directory, wasm file stem)
CONTRACTS = {
    "attester-registry": ("contracts/attester-registry", "attester_registry"),
    "attestation-registry": ("contracts/attestation-registry", "attestation_registry"),
    "multisig-account": ("contracts/multisig-account", "multisig_account"),
}

WASM_DIR = ROOT / "target" / "wasm32v1-none" / "release"


def run_git(*args):
    return subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.strip()


def workspace_version():
    with open(ROOT / "Cargo.toml", "rb") as f:
        data = tomllib.load(f)
    return data["workspace"]["package"]["version"]


def storage_schema_version(crate_dir):
    lib_rs = ROOT / crate_dir / "src" / "lib.rs"
    text = lib_rs.read_text(encoding="utf-8")
    m = re.search(r"const\s+SCHEMA_VERSION\s*:\s*u32\s*=\s*(\d+)", text)
    return int(m.group(1)) if m else None


def wasm_info(stem):
    wasm_path = WASM_DIR / f"{stem}.wasm"
    if not wasm_path.is_file():
        return {"target": "wasm32v1-none", "optimize": False, "sha256": None, "size_bytes": None}
    data = wasm_path.read_bytes()
    return {
        "target": "wasm32v1-none",
        "optimize": False,
        "sha256": hashlib.sha256(data).hexdigest(),
        "size_bytes": len(data),
    }


def build_contracts():
    contracts = []
    for name, (crate_dir, stem) in CONTRACTS.items():
        contracts.append(
            {
                "name": name,
                "crate_path": crate_dir,
                "package_version": workspace_version(),
                "storage_schema_version": storage_schema_version(crate_dir),
                "wasm": wasm_info(stem),
            }
        )
    return contracts


def build_bindings(contracts_by_name):
    bindings = []
    bindings_dir = ROOT / "bindings"
    if not bindings_dir.is_dir():
        return bindings
    for pkg_dir in sorted(bindings_dir.iterdir()):
        package_json = pkg_dir / "package.json"
        if not package_json.is_file():
            continue
        pkg = json.loads(package_json.read_text(encoding="utf-8"))
        contract_name = pkg_dir.name
        contract = contracts_by_name.get(contract_name)
        bindings.append(
            {
                "contract": contract_name,
                "language": "typescript",
                "package_name": pkg.get("name", contract_name),
                "package_version": pkg.get("version", "0.0.0"),
                "path": str(pkg_dir.relative_to(ROOT)),
                "generated_from_wasm_sha256": contract["wasm"]["sha256"] if contract else None,
            }
        )
    return bindings


EVENT_RE = re.compile(
    r"#\[contractevent\]\s*(?:#\[[^\]]*\]\s*)*pub struct (\w+)\s*\{(.*?)\n\}",
    re.DOTALL,
)
FIELD_RE = re.compile(r"(#\[topic\]\s*)?pub\s+(\w+)\s*:")


def parse_events(crate_dir, contract_name):
    lib_rs = ROOT / crate_dir / "src" / "lib.rs"
    text = lib_rs.read_text(encoding="utf-8")
    events = []
    for name, body in EVENT_RE.findall(text):
        topic_fields, data_fields = [], []
        for is_topic, field in FIELD_RE.findall(body):
            (topic_fields if is_topic else data_fields).append(field)
        events.append(
            {
                "contract": contract_name,
                "name": name,
                "topic_fields": topic_fields,
                "data_fields": data_fields,
            }
        )
    return events


def classify_events(events, previous_events_by_key):
    for ev in events:
        key = (ev["contract"], ev["name"])
        prev = previous_events_by_key.get(key)
        if previous_events_by_key == {}:
            ev["compatibility"] = "unclassified"
        elif prev is None:
            ev["compatibility"] = "new"
        elif prev["topic_fields"] == ev["topic_fields"] and set(prev["data_fields"]) <= set(
            ev["data_fields"]
        ):
            # Topics are positional and part of the event's identity/ABI: any
            # change there is breaking. Data fields may grow (new optional
            # data) without breaking an existing decoder that reads by name.
            ev["compatibility"] = "compatible"
        else:
            ev["compatibility"] = "breaking"
    return events


def build_events(previous_manifest):
    previous_events_by_key = {}
    if previous_manifest is not None:
        previous_events_by_key = {
            (e["contract"], e["name"]): e for e in previous_manifest.get("events", [])
        }
    events = []
    for name, (crate_dir, _stem) in CONTRACTS.items():
        events.extend(parse_events(crate_dir, name))
    classify_events(events, previous_events_by_key)
    if previous_manifest is not None:
        current_keys = {(e["contract"], e["name"]) for e in events}
        for key, prev in previous_events_by_key.items():
            if key not in current_keys:
                events.append({**prev, "compatibility": "removed"})
    return events


def build_deployments(contracts_by_name):
    networks_toml = ROOT / "config" / "networks.toml"
    with open(networks_toml, "rb") as f:
        networks = tomllib.load(f)
    deployments = []
    for network_name, network in networks.items():
        for contract_name, contract_id in network.get("contracts", {}).items():
            # networks.toml uses snake_case keys; manifest contract names are
            # kebab-case to match crates/ and bindings/ directory names.
            name = contract_name.replace("_", "-")
            contract = contracts_by_name.get(name)
            deployments.append(
                {
                    "network": network_name,
                    "contract": name,
                    "contract_id": contract_id or None,
                    # networks.toml only records the current contract ID, not
                    # the wasm hash or when it was deployed — see ADR-0010
                    # Follow-up for the proposed append-only deployment ledger
                    # that would let this be populated automatically.
                    "wasm_sha256": None,
                    "storage_schema_version": contract["storage_schema_version"] if contract else None,
                    "deployed_at": None,
                    "upgrade_tx": None,
                    "previous_wasm_sha256": None,
                    "status": "deployed" if contract_id else "not_deployed",
                    "source": "config/networks.toml (current pointer only, no historical ledger yet)",
                }
            )
    return deployments


def build_compatibility():
    return {
        "policy_ref": "docs/adr/0010-release-manifest-and-compatibility.md#compatibility-policy",
        "consumers": [
            {
                "repo": "lafiya-web",
                "requirements_ref": "docs/release-manifest/examples/lafiya-web.requirements.json",
            }
        ],
    }


def generate(previous_manifest):
    contracts = build_contracts()
    contracts_by_name = {c["name"]: c for c in contracts}
    try:
        git_tag = run_git("describe", "--tags", "--exact-match", "HEAD")
    except subprocess.CalledProcessError:
        git_tag = None
    return {
        "manifest_version": 1,
        "release": {
            "workspace_version": workspace_version(),
            "git_commit": run_git("rev-parse", "HEAD"),
            "git_tag": git_tag,
            "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        },
        "contracts": contracts,
        "bindings": build_bindings(contracts_by_name),
        "events": build_events(previous_manifest),
        "deployments": build_deployments(contracts_by_name),
        "compatibility": build_compatibility(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--previous", type=Path, help="prior release manifest, used to classify event compatibility"
    )
    parser.add_argument("-o", "--output", type=Path, help="write to this path instead of stdout")
    parser.add_argument("--pretty", action="store_true", help="indent the JSON output")
    args = parser.parse_args()

    previous_manifest = None
    if args.previous:
        previous_manifest = json.loads(args.previous.read_text(encoding="utf-8"))

    manifest = generate(previous_manifest)
    text = json.dumps(manifest, indent=2 if args.pretty else None, sort_keys=False)
    if args.output:
        args.output.write_text(text + "\n", encoding="utf-8")
    else:
        print(text)


if __name__ == "__main__":
    main()
