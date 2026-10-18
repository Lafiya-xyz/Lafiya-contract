#!/usr/bin/env python3
"""Verify a key-ceremony record against the chain.

Checks that the admin and guardian `multisig-account` contracts hold exactly
the public keys and threshold the ceremony recorded. Also checks that the
registries point at them: `get_admin` is the admin account, and the guardian
account holds the `Guardian` role. See docs/runbooks/key-ceremony.md.

The signer-set hash binds a threshold to an ordered key list. The key list is
in the canonical order that `get_signers` returns (key type, then ascending
key bytes):

    sha256("lafiya-signer-set-v1" || u32_be(threshold) || (0x00 || ed25519_key)*)

Witnesses compare the hash read aloud at the ceremony, the hash in the
record, and the hash recomputed from chain state.

Usage:
    scripts/verify_signer_set.py hash --threshold N G...      # during the ceremony
    scripts/verify_signer_set.py verify RECORD.json [--offline] [--source ACCOUNT]

`verify` without --offline reads chain state with read-only (`--send=no`)
`stellar contract invoke` calls. --source is any identity or G-address the
CLI can simulate as (default: $STELLAR_ACCOUNT).
"""

import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any

DOMAIN = b"lafiya-signer-set-v1"
ED25519_TAG = b"\x00"
ACCOUNT_VERSION = 6 << 3  # strkey version byte for G... account ids

Invoke = Callable[[str, str], Any]


def crc16_xmodem(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = (crc << 1) ^ 0x1021 if crc & 0x8000 else crc << 1
            crc &= 0xFFFF
    return crc


def decode_account(strkey: str) -> bytes:
    """G... strkey -> 32-byte ed25519 key; raises ValueError on a bad checksum."""
    raw = base64.b32decode(strkey)
    if len(raw) != 35 or raw[0] != ACCOUNT_VERSION:
        raise ValueError(f"{strkey}: not an account (G...) strkey")
    payload, checksum = raw[:-2], raw[-2:]
    if crc16_xmodem(payload).to_bytes(2, "little") != checksum:
        raise ValueError(f"{strkey}: checksum mismatch (typo?)")
    return payload[1:]


def encode_account(key: bytes) -> str:
    payload = bytes([ACCOUNT_VERSION]) + key
    return base64.b32encode(payload + crc16_xmodem(payload).to_bytes(2, "little")).decode()


def signer_set_hash(threshold: int, keys: list[bytes]) -> str:
    body = b"".join(ED25519_TAG + key for key in sorted(keys))
    return hashlib.sha256(DOMAIN + threshold.to_bytes(4, "big") + body).hexdigest()


class Checks:
    def __init__(self) -> None:
        self.failed = 0

    def check(self, ok: bool, what: str, detail: str = "") -> bool:
        print(f"[{'PASS' if ok else 'FAIL'}] {what}" + (f": {detail}" if detail and not ok else ""))
        self.failed += 0 if ok else 1
        return ok


def offline_checks(record: dict[str, Any], c: Checks) -> dict[str, list[bytes]]:
    keys_by_account: dict[str, list[bytes]] = {}
    for name, acct in record["accounts"].items():
        keys = []
        for seat in acct["signers"]:
            try:
                keys.append(decode_account(seat["public_key"]))
            except ValueError as exc:
                c.check(False, f"{name}: {seat['seat']} public key", str(exc))
        keys_by_account[name] = keys
        n, t = len(acct["signers"]), acct["threshold"]
        c.check(len(set(keys)) == len(keys), f"{name}: no duplicate keys")
        minimum = 2 if name == "admin" else 1
        c.check(
            minimum <= t < n,
            f"{name}: threshold {t}-of-{n} within policy ({minimum} <= t < n)",
            "one lost key must not lock the account, and admin must need two people",
        )
        computed = signer_set_hash(t, keys)
        c.check(
            computed == acct["signer_set_hash"],
            f"{name}: recorded signer-set hash matches keys",
            f"recomputed {computed}",
        )
    names = list(keys_by_account)
    for i, a in enumerate(names):
        for b in names[i + 1 :]:
            shared = set(keys_by_account[a]) & set(keys_by_account[b])
            c.check(
                not shared,
                f"{a} and {b} share no keys",
                ", ".join(encode_account(k) for k in shared),
            )
    return keys_by_account


def chain_keys(signers: list[Any]) -> list[bytes]:
    keys = []
    for entry in signers:
        if not isinstance(entry, dict) or set(entry) != {"Ed25519"}:
            raise ValueError(f"unsupported signer {entry!r}; only Ed25519 ceremony keys")
        keys.append(bytes.fromhex(entry["Ed25519"]))
    return keys


def online_checks(record: dict[str, Any], invoke: Invoke, c: Checks) -> None:
    for name, acct in record["accounts"].items():
        cid, threshold = acct["contract_id"], acct["threshold"]
        try:
            keys = chain_keys(invoke(cid, "get_signers"))
        except ValueError as exc:
            c.check(False, f"{name}: on-chain signers", str(exc))
            continue
        want = {s["public_key"] for s in acct["signers"]}
        got = {encode_account(k) for k in keys}
        c.check(
            got == want,
            f"{name}: on-chain signer set equals the ceremony record",
            f"missing {sorted(want - got)}, unexpected {sorted(got - want)}",
        )
        on_chain_t = invoke(cid, "get_threshold")
        c.check(
            on_chain_t == threshold, f"{name}: on-chain threshold is {threshold}", str(on_chain_t)
        )
        policy = invoke(cid, "get_policy")
        plain = (
            all(s["weight"] == 1 and s["role"] is None for s in policy["signers"])
            and not policy["role_requirements"]
            and policy["weight_threshold"] == policy["minimum_signers"] == threshold
        )
        c.check(plain, f"{name}: policy is plain {threshold}-of-n (no weights or roles)")
        c.check(
            signer_set_hash(int(on_chain_t), keys) == acct["signer_set_hash"],
            f"{name}: signer-set hash recomputed from chain matches",
        )
    for binding in record.get("bindings", []):
        cid, label = binding["contract_id"], binding["label"]
        if "admin" in binding:
            admin_id = record["accounts"][binding["admin"]]["contract_id"]
            got = invoke(cid, "get_admin")
            c.check(got == admin_id, f"{label}: admin is the {binding['admin']} account", got)
        if "guardian" in binding:
            guardian_id = record["accounts"][binding["guardian"]]["contract_id"]
            has = invoke(cid, f"has_role --role Guardian --account {guardian_id}")
            c.check(has is True, f"{label}: {binding['guardian']} account holds Guardian")


def stellar_invoke(network: str, source: str) -> Invoke:
    def invoke(contract: str, call: str) -> Any:
        cmd = ["stellar", "contract", "invoke", "--id", contract, "--network", network]
        cmd += ["--source", source, "--send=no", "--", *call.split()]
        proc = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(proc.stdout)

    return invoke


def main(argv: list[str] | None = None, invoke: Invoke | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    h = sub.add_parser("hash", help="print the signer-set hash for a key list")
    h.add_argument("--threshold", type=int, required=True)
    h.add_argument("keys", nargs="+")
    v = sub.add_parser("verify", help="check a ceremony record (and the chain)")
    v.add_argument("record", type=Path)
    v.add_argument("--offline", action="store_true")
    v.add_argument("--source", default=os.environ.get("STELLAR_ACCOUNT"))
    args = parser.parse_args(argv)

    if args.command == "hash":
        print(signer_set_hash(args.threshold, [decode_account(k) for k in args.keys]))
        return 0

    record = json.loads(args.record.read_text())
    c = Checks()
    print(f"ceremony {record['ceremony']} on {record['network']}")
    offline_checks(record, c)
    if not args.offline:
        if invoke is None:
            if not args.source:
                parser.error("--source (or $STELLAR_ACCOUNT) is required to read the chain")
            invoke = stellar_invoke(record["network"], args.source)
        online_checks(record, invoke, c)
    print(f"{c.failed} check(s) failed" if c.failed else "all checks passed")
    return 1 if c.failed else 0


if __name__ == "__main__":
    sys.exit(main())
