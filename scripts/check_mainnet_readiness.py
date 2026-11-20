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
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVIDENCE = ("audit_report", "ceremony_record", "game_day_report", "monitoring_evidence")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
REHEARSAL_MAX_AGE_DAYS = 14
REHEARSAL_REQUIRED_SCENARIOS = (
    "happy_path",
    "no_trustline",
    "unauthorized_trustline",
    "merged_account",
    "paused_pool",
    "drift",
)


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
    errors.extend(validate_rehearsal(data.get("testnet_rehearsal"), root))
    return errors


def validate_rehearsal(entry: object, root: Path) -> list[str]:
    """Require a recent, green testnet rehearsal report referenced by the manifest."""
    if not isinstance(entry, dict):
        return ["testnet_rehearsal: missing; mainnet requires a recent green testnet rehearsal"]
    errors = []
    path, digest = entry.get("path"), entry.get("sha256")
    if not isinstance(path, str) or not isinstance(digest, str) or not SHA256.match(digest):
        errors.append("testnet_rehearsal: needs string `path` and lowercase hex `sha256`")
        return errors
    target = (root / path).resolve()
    if root.resolve() not in target.parents or not target.is_file():
        return [f"testnet_rehearsal: `{path}` is not a file inside the repository"]
    if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
        errors.append("testnet_rehearsal: sha256 of the report does not match the manifest")
    try:
        report = json.loads(target.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        return errors + [f"testnet_rehearsal: report is invalid JSON: {e}"]
    if not isinstance(report, dict):
        return errors + ["testnet_rehearsal: report must be a JSON object"]
    if report.get("status") != "green":
        errors.append(f"testnet_rehearsal: report status is {report.get('status')!r}, expected 'green'")
    finished = report.get("finished_at")
    try:
        finished_at = datetime.fromisoformat(str(finished).replace("Z", "+00:00"))
    except ValueError:
        errors.append("testnet_rehearsal: report needs an ISO-8601 `finished_at` timestamp")
    else:
        if finished_at.tzinfo is None:
            errors.append("testnet_rehearsal: `finished_at` must include a timezone")
        else:
            age = datetime.now(timezone.utc) - finished_at
            if age > timedelta(days=REHEARSAL_MAX_AGE_DAYS):
                errors.append(
                    f"testnet_rehearsal: report is stale ({age.days} days old, "
                    f"max {REHEARSAL_MAX_AGE_DAYS}); rerun the nightly rehearsal"
                )
    scenarios = report.get("scenarios")
    passed = set()
    if isinstance(scenarios, list):
        for s in scenarios:
            if isinstance(s, dict) and s.get("status") == "pass" and isinstance(s.get("name"), str):
                passed.add(s["name"])
    missing = [name for name in REHEARSAL_REQUIRED_SCENARIOS if name not in passed]
    if missing:
        errors.append(f"testnet_rehearsal: scenarios not passing: {', '.join(missing)}")
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
