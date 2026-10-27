#!/usr/bin/env python3
"""Block enabling mainnet in config/networks.toml unless config/mainnet-readiness.json validates.

Mainnet counts as enabled when any [mainnet.contracts] id is non-empty. The manifest
schema is documented in docs/runbooks/mainnet-readiness.md.
"""
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVIDENCE = ("audit_report", "ceremony_record", "game_day_report", "monitoring_evidence")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def mainnet_enabled(networks: Path) -> bool:
    cfg = tomllib.loads(networks.read_text(encoding="utf-8"))
    contracts = cfg.get("mainnet", {}).get("contracts", {})
    return any(str(v).strip() for v in contracts.values())


def validate_manifest(manifest: Path, root: Path) -> list[str]:
    if not manifest.is_file():
        return [f"{manifest.name}: missing; mainnet cannot be enabled without readiness evidence"]
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        return [f"{manifest.name}: invalid JSON: {e}"]
    errors = []
    for key in EVIDENCE:
        entry = data.get(key)
        if not isinstance(entry, dict):
            errors.append(f"{key}: missing")
            continue
        path, digest = entry.get("path"), entry.get("sha256")
        if not isinstance(path, str) or not isinstance(digest, str) or not SHA256.match(digest):
            errors.append(f"{key}: needs string `path` and lowercase hex `sha256`")
            continue
        target = (root / path).resolve()
        if root.resolve() not in target.parents or not target.is_file():
            errors.append(f"{key}: `{path}` is not a file inside the repository")
        elif hashlib.sha256(target.read_bytes()).hexdigest() != digest:
            errors.append(f"{key}: sha256 of `{path}` does not match the manifest")
    signoff = data.get("owner_signoff")
    if not isinstance(signoff, list) or not signoff or not all(isinstance(s, str) and s for s in signoff):
        errors.append("owner_signoff: must be a non-empty list of owner handles")
    return errors


def check(root: Path = ROOT) -> list[str]:
    networks = root / "config" / "networks.toml"
    if not mainnet_enabled(networks):
        return []
    return validate_manifest(root / "config" / "mainnet-readiness.json", root)


if __name__ == "__main__":
    errs = check()
    for e in errs:
        print(f"error: {e}", file=sys.stderr)
    sys.exit(1 if errs else 0)
