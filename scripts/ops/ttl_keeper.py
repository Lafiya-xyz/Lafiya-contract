#!/usr/bin/env python3
"""TTL keeper: extend critical contract entries before archival, within a spend cap.

For each watched entry the keeper reads liveUntilLedgerSeq via RPC, alerts when headroom is
below `alert_below_ledgers`, and extends entries below `extend_below_ledgers` while the daily
spend cap allows. State (spend ledger) lives in a flock-guarded file, and every entry is
re-read under the lock, so concurrent instances never extend the same entry twice.
Runbook: docs/runbooks/ttl-keeper.md
"""
import argparse
import fcntl
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


class RpcClient:
    def __init__(self, url: str):
        self.url = url

    def live_until(self, entry: dict) -> tuple[int, int]:
        """Return (latest_ledger, live_until_ledger) for a watchlist entry."""
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "getLedgerEntries",
                           "params": {"keys": [entry["key_xdr"]]}}).encode()
        req = urllib.request.Request(self.url, body, {"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=30) as resp:  # noqa: S310 - operator-configured URL
            result = json.load(resp)["result"]
        if not result.get("entries"):
            raise RuntimeError(f"{entry['id']}: entry not found (already archived?)")
        return result["latestLedger"], result["entries"][0]["liveUntilLedgerSeq"]


def stellar_extend(entry: dict, ledgers: int, source: str) -> int:
    """Extend via the stellar CLI using a named identity (no secrets handled here)."""
    cmd = ["stellar", "contract", "extend", "--id", entry["contract"], "--durability", "persistent",
           "--ledgers-to-extend", str(ledgers), "--source-account", source]
    if entry.get("key"):
        cmd += ["--key", entry["key"]]
    subprocess.run(cmd, check=True, capture_output=True, text=True)
    return entry.get("est_cost_stroops", 0)


def today() -> str:
    return time.strftime("%Y-%m-%d", time.gmtime())


def run(cfg: dict, rpc, extend, state_path: Path, dry_run: bool, log) -> int:
    """Return exit code: 0 healthy, 2 if any entry is (still) below the alert threshold."""
    alerts = 0
    state_path.touch(exist_ok=True)
    with open(state_path, "r+") as fh:
        fcntl.flock(fh, fcntl.LOCK_EX)
        raw = fh.read()
        state = json.loads(raw) if raw else {}
        if state.get("day") != today():
            state = {"day": today(), "spent_stroops": 0}
        for entry in cfg["entries"]:
            latest, live_until = rpc.live_until(entry)
            headroom = live_until - latest
            if headroom < cfg["extend_below_ledgers"]:
                cost = entry.get("est_cost_stroops", 0)
                if state["spent_stroops"] + cost > cfg["daily_cap_stroops"]:
                    log({"event": "cap_reached", "entry": entry["id"], "spent": state["spent_stroops"]})
                elif dry_run:
                    log({"event": "would_extend", "entry": entry["id"], "headroom": headroom})
                else:
                    state["spent_stroops"] += extend(entry, cfg["extend_to_ledgers"], cfg["source_identity"]) or cost
                    log({"event": "extended", "entry": entry["id"], "spent": state["spent_stroops"]})
                    latest, live_until = rpc.live_until(entry)
                    headroom = live_until - latest
            if headroom < cfg["alert_below_ledgers"]:
                alerts += 1
                log({"event": "ALERT", "entry": entry["id"], "headroom_ledgers": headroom})
        fh.seek(0)
        fh.truncate()
        json.dump(state, fh)
    return 2 if alerts else 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--config", required=True, type=Path)
    p.add_argument("--rpc-url", required=True)
    p.add_argument("--state", type=Path, default=Path("ttl-keeper-state.json"))
    p.add_argument("--dry-run", action="store_true")
    a = p.parse_args()
    cfg = json.loads(a.config.read_text(encoding="utf-8"))
    log = lambda rec: print(json.dumps({"ts": int(time.time()), **rec}), flush=True)  # noqa: E731
    return run(cfg, RpcClient(a.rpc_url), stellar_extend, a.state, a.dry_run, log)


if __name__ == "__main__":
    sys.exit(main())
