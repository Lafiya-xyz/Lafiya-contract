#!/usr/bin/env python3
"""Reproducible, signed publication snapshots for the transparency dashboard.

  snapshot.py sign   --data DIR --key PRIV.pem       writes DIR/snapshot.sig
  snapshot.py verify --data DIR --pubkey PUB.pem [--regenerate -- GENERATE_ARGS]

`snapshot.json` (written by generate.py) records the ledger height, a hash of
the source events (the index version) and a content hash over every published
data file. It is signed with an Ed25519 key through the `openssl` CLI (3.x), so
no Python dependencies are needed. See VERIFY.md.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA = 1
UNSIGNED = {"snapshot.json", "snapshot.sig"}


def sha256_hex(data):
    return hashlib.sha256(data).hexdigest()


def index_version(events):
    """Hash of the exact events the snapshot was computed from."""
    canonical = "\n".join(json.dumps(e, sort_keys=True, separators=(",", ":")) for e in events)
    return sha256_hex(canonical.encode())


def content_hash(data_dir):
    lines = []
    for name in sorted(os.listdir(data_dir)):
        if name in UNSIGNED:
            continue
        with open(os.path.join(data_dir, name), "rb") as f:
            lines.append(f"{sha256_hex(f.read())}  {name}\n")
    return sha256_hex("".join(lines).encode())


def write_manifest(data_dir, ledger_height, events):
    manifest = {
        "schema": SCHEMA,
        "ledger_height": ledger_height,
        "index_version": index_version(events),
        "content_hash": content_hash(data_dir),
    }
    with open(os.path.join(data_dir, "snapshot.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    return manifest


def sign(data_dir, key):
    subprocess.run(
        ["openssl", "pkeyutl", "-sign", "-inkey", key, "-rawin",
         "-in", os.path.join(data_dir, "snapshot.json"),
         "-out", os.path.join(data_dir, "snapshot.sig")],
        check=True)


def verify(data_dir, pubkey, regenerate=None):
    """Return a list of problems; empty means the snapshot is valid."""
    problems = []
    with open(os.path.join(data_dir, "snapshot.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    if content_hash(data_dir) != manifest["content_hash"]:
        problems.append("content hash does not match the data files")
    sig = subprocess.run(
        ["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", pubkey, "-rawin",
         "-in", os.path.join(data_dir, "snapshot.json"),
         "-sigfile", os.path.join(data_dir, "snapshot.sig")],
        capture_output=True)
    if sig.returncode != 0:
        problems.append("signature is invalid for this public key")
    if regenerate is not None:
        sys.path.insert(0, HERE)
        import generate
        with tempfile.TemporaryDirectory() as tmp:
            generate.main(regenerate + ["--out", tmp])
            with open(os.path.join(tmp, "data", "snapshot.json"), encoding="utf-8") as f:
                mine = json.load(f)
        for field in ("ledger_height", "index_version", "content_hash"):
            if mine[field] != manifest[field]:
                problems.append(f"recomputed {field} differs from the published snapshot")
    return problems


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sign")
    s.add_argument("--data", required=True)
    s.add_argument("--key", required=True)
    v = sub.add_parser("verify")
    v.add_argument("--data", required=True)
    v.add_argument("--pubkey", required=True)
    v.add_argument("--regenerate", nargs=argparse.REMAINDER,
                   help="after `--`: generate.py arguments (minus --out) to recompute from public events")
    args = p.parse_args(argv)
    if args.cmd == "sign":
        sign(args.data, args.key)
        return 0
    regen = [a for a in (args.regenerate or []) if a != "--"] or None
    problems = verify(args.data, args.pubkey, regen)
    for m in problems:
        print("FAIL:", m, file=sys.stderr)
    print("snapshot OK" if not problems else "snapshot INVALID")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
