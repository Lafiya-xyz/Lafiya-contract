#!/usr/bin/env python3
"""Synthetic public-read-path probes for the status page (docs/status-page.md).

Probes: rpc liveness, contract view call, indexer lag, fixture verification.
Appends one result record to a JSONL history; never uses secrets or writes.
"""

import argparse
import json
import sys
import time
import urllib.request


def post(url, body):
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.load(r)


def get(url):
    with urllib.request.urlopen(url, timeout=15) as r:
        return json.load(r)


def timed(name, fn):
    t = time.time()
    try:
        detail = fn()
        return {"probe": name, "ok": True, "ms": int((time.time() - t) * 1000), "detail": detail}
    except Exception as e:  # a probe failure is a result, not a crash
        return {"probe": name, "ok": False, "ms": int((time.time() - t) * 1000), "detail": str(e)}


def run(a):
    def rpc():
        r = post(a.rpc_url, {"jsonrpc": "2.0", "id": 1, "method": "getHealth"})
        assert r["result"]["status"] == "healthy", r
        return r["result"]["latestLedger"]

    def indexer():
        h = get(a.indexer_url.rstrip("/") + "/health")
        assert h.get("lag_ledgers", 0) <= a.max_lag, h
        return h.get("lag_ledgers")

    def verify():
        r = get(a.indexer_url.rstrip("/") + "/attestations?record_hash=" + a.fixture_hash)
        assert r["items"], "fixture attestation not found"
        return len(r["items"])

    def pool_invariants():
        # Read-only self-diagnostic view on the incentive pool; reports solvency
        # and accounting invariants (see incentive-pool check_invariants()).
        r = post(a.rpc_url, {
            "jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
            "params": {"contract_id": a.pool_contract_id, "function": "check_invariants", "args": []},
        })
        report = r["result"]
        assert report.get("ok", False), report
        return {"ok": report.get("ok"), "violations": report.get("violations", [])}

    probes = [timed("rpc_liveness", rpc), timed("indexer_lag", indexer), timed("verify_fixture", verify),
              timed("pool_invariants", pool_invariants)]
    return {"ts": int(time.time()), "location": a.location, "deployment": a.deployment, "probes": probes}


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--deployment", required=True)
    p.add_argument("--location", required=True)
    p.add_argument("--rpc-url", required=True)
    p.add_argument("--indexer-url", required=True)
    p.add_argument("--fixture-hash", required=True)
    p.add_argument("--pool-contract-id", required=True)
    p.add_argument("--max-lag", type=int, default=20)
    p.add_argument("--history", required=True)
    a = p.parse_args(argv)
    rec = run(a)
    with open(a.history, "a", encoding="utf-8") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
    return 0 if all(x["ok"] for x in rec["probes"]) else 1


if __name__ == "__main__":
    sys.exit(main())
