#!/usr/bin/env python3
"""Minimal indexer-style client used by the chaos scenarios.

Polls getLatestLedger then getEvents and fails closed (exit 2, no partial
output) when the RPC misbehaves: bad JSON, HTTP errors, a tip that went
backwards, a ledger gap, or an implausible result.
"""
import json
import sys
import urllib.request

EXPECTED_TIP = 5000


def call(url, method, params=None):
    req = urllib.request.Request(
        url, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}).encode(),
        {"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=5).read())["result"]


def main(url):
    tip = call(url, "getLatestLedger")["sequence"]
    if tip < EXPECTED_TIP - 10:
        raise ValueError(f"stale tip {tip}")
    start = tip - 3
    ledgers = [e["ledger"] for e in call(url, "getEvents", {"startLedger": start})["events"]]
    if ledgers != list(range(start, tip + 1)):
        raise ValueError(f"ledger gap: got {ledgers}")
    print("synced", tip)


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except Exception as e:  # fail closed on any RPC anomaly
        print("probe rejected RPC:", type(e).__name__, e, file=sys.stderr)
        sys.exit(2)
