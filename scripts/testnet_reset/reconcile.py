#!/usr/bin/env python3
"""Write freshly deployed contract IDs back into repository configuration.

    python3 scripts/testnet_reset/reconcile.py deployments/testnet.json [--network testnet]

Updates `[<network>.contracts]` in config/networks.toml and, when present, the
`<network>` entry of each binding's `export const networks` constant.
"""
import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NETWORKS_TOML = REPO_ROOT / "config" / "networks.toml"
BINDINGS = {
    "attester_registry": REPO_ROOT / "bindings" / "attester-registry" / "src" / "index.ts",
    "attestation_registry": REPO_ROOT / "bindings" / "attestation-registry" / "src" / "index.ts",
}


def update_toml(text, network, ids):
    section = re.search(
        rf"^\[{re.escape(network)}\.contracts\]\n(.*?)(?=^\[|\Z)",
        text,
        re.DOTALL | re.MULTILINE,
    )
    if not section:
        raise SystemExit(f"[{network}.contracts] not found in {NETWORKS_TOML}")
    body = section.group(1)
    for key, value in ids.items():
        body = re.sub(rf'^{key} = ".*"$', f'{key} = "{value}"', body, flags=re.MULTILINE)
    return text[: section.start(1)] + body + text[section.end(1) :]


def update_binding(text, network, contract_id):
    return re.sub(
        rf'({network}:\s*\{{[^}}]*?contractId:\s*")[^"]*(")',
        rf"\g<1>{contract_id}\g<2>",
        text,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("deployment", type=Path)
    parser.add_argument("--network", default="testnet")
    args = parser.parse_args()

    deployment = json.loads(args.deployment.read_text())
    ids = {k: deployment[k] for k in BINDINGS}

    NETWORKS_TOML.write_text(update_toml(NETWORKS_TOML.read_text(), args.network, ids))
    for key, path in BINDINGS.items():
        if path.exists() and "export const networks" in path.read_text():
            path.write_text(update_binding(path.read_text(), args.network, ids[key]))
    print(f"reconciled {args.network}: {ids}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
