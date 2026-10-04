#!/usr/bin/env python3
"""Detect Stellar testnet resets and missing contract instances (issue #425).

    python3 scripts/testnet_reset/detect.py [--network testnet] [--update-state]

Prints one JSON object: {"status": "healthy" | "contract-missing" | "network-reset", ...}
and, when running under GitHub Actions, writes `status=<status>` to $GITHUB_OUTPUT.

Detection:
  * `getLatestLedger` is compared with the last recorded sequence in
    config/testnet-state.json. SDF resets restart the ledger from genesis, so a
    sharp drop (below RESET_RATIO of the recorded value) means `network-reset`.
    A changed passphrase from `getNetwork` also means `network-reset`.
  * Each configured contract ID in config/networks.toml is checked with
    `getLedgerEntries` on its contract-instance key. Any missing instance on an
    otherwise healthy network means `contract-missing`.

Stdlib only, so it runs in CI without extra dependencies.
"""
import argparse
import base64
import json
import os
import struct
import sys
import tomllib
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NETWORKS_TOML = REPO_ROOT / "config" / "networks.toml"
STATE_FILE = REPO_ROOT / "config" / "testnet-state.json"

HEALTHY = "healthy"
CONTRACT_MISSING = "contract-missing"
NETWORK_RESET = "network-reset"

# A reset restarts from ledger 1; normal operation only ever increases the
# sequence. Anything below half the recorded value is treated as a reset.
RESET_RATIO = 0.5

_B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"


def contract_id_to_hash(contract_id):
    """Decode a `C...` strkey to its 32-byte contract hash."""
    bits = 0
    nbits = 0
    out = bytearray()
    for ch in contract_id.strip():
        bits = (bits << 5) | _B32.index(ch)
        nbits += 5
        if nbits >= 8:
            nbits -= 8
            out.append((bits >> nbits) & 0xFF)
    # 1 version byte + 32 payload bytes + 2 checksum bytes
    if len(out) != 35 or out[0] != 2 << 3:
        raise ValueError(f"not a contract strkey: {contract_id}")
    return bytes(out[1:33])


def instance_ledger_key(contract_id):
    """Base64 XDR LedgerKey for a contract's instance entry."""
    xdr = struct.pack(">i", 6)  # LedgerEntryType CONTRACT_DATA
    xdr += struct.pack(">i", 1)  # ScAddressType CONTRACT
    xdr += contract_id_to_hash(contract_id)
    xdr += struct.pack(">i", 20)  # ScValType LEDGER_KEY_CONTRACT_INSTANCE
    xdr += struct.pack(">i", 1)  # ContractDataDurability PERSISTENT
    return base64.b64encode(xdr).decode()


def classify(state, latest_ledger, network, entries_response, contract_ids):
    """Pure classification over RPC responses. `state` may be None (first run)."""
    sequence = latest_ledger["sequence"]
    if state:
        if network.get("passphrase") != state.get("passphrase"):
            return NETWORK_RESET, "network passphrase changed"
        if sequence < state["latest_ledger"] * RESET_RATIO:
            return (
                NETWORK_RESET,
                f"ledger sequence dropped from {state['latest_ledger']} to {sequence}",
            )
    found = {e["key"] for e in entries_response.get("entries") or []}
    missing = [cid for cid in contract_ids if instance_ledger_key(cid) not in found]
    if missing:
        return CONTRACT_MISSING, "missing contract instances: " + ", ".join(missing)
    return HEALTHY, "all configured contract instances present"


def rpc(url, method, params=None):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}
    ).encode()
    req = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        payload = json.load(resp)
    if "error" in payload:
        raise RuntimeError(f"{method}: {payload['error']}")
    return payload["result"]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--network", default="testnet")
    parser.add_argument(
        "--update-state",
        action="store_true",
        help="record the current ledger and passphrase in config/testnet-state.json",
    )
    args = parser.parse_args()

    cfg = tomllib.loads(NETWORKS_TOML.read_text())[args.network]
    url = cfg["rpc_url"]
    contract_ids = [cid for cid in cfg.get("contracts", {}).values() if cid]

    state = json.loads(STATE_FILE.read_text()) if STATE_FILE.exists() else None
    latest = rpc(url, "getLatestLedger")
    network = rpc(url, "getNetwork")
    entries = (
        rpc(url, "getLedgerEntries", {"keys": [instance_ledger_key(c) for c in contract_ids]})
        if contract_ids
        else {"entries": []}
    )

    status, reason = classify(state, latest, network, entries, contract_ids)
    print(json.dumps({"status": status, "reason": reason, "latest_ledger": latest["sequence"]}))

    if args.update_state:
        STATE_FILE.write_text(
            json.dumps(
                {
                    "passphrase": network.get("passphrase"),
                    "protocol_version": latest.get("protocolVersion"),
                    "latest_ledger": latest["sequence"],
                },
                indent=2,
            )
            + "\n"
        )

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a") as fh:
            fh.write(f"status={status}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
