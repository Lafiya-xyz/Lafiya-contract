#!/usr/bin/env python3
"""Read-only conformance probes against public Soroban RPC providers (issue #749).

For each provider it records a capability measurement (latency, ledger, which
read-only methods answer and the JSON shape of their results), diffs the
measurement against the previous run, and writes:

  <out>/measurements.json   capability measurements (CI artifact)
  <out>/drift.md            human-readable drift report (empty if no drift)
  <out>/drift.json          machine-readable drift summary

Exit code is 0 when no drift was detected, 1 when drift was detected, and 2
when every provider failed (so a total outage is not reported as drift).
Never signs, submits, or writes anything on-chain.
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

# Read-only methods probed on every provider. `getNetwork` and `getHealth` are
# cheap liveness/shape checks; `getLatestLedger` and `getLedgerEntries` cover
# the read paths the CLI and indexer depend on.
PROBES = [
    ("getHealth", {}),
    ("getNetwork", {}),
    ("getLatestLedger", {}),
    ("getLedgerEntries", {"keys": []}),
]

# Fields whose presence/type changes are treated as schema drift.
SHAPE_DEPTH = 3


def post(url, body, timeout):
    req = urllib.request.Request(
        url, json.dumps(body).encode(), {"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def shape(value, depth=0):
    """Structural fingerprint of a JSON value: keys and types, not values."""
    if depth >= SHAPE_DEPTH:
        return type(value).__name__
    if isinstance(value, dict):
        return {k: shape(v, depth + 1) for k, v in sorted(value.items())}
    if isinstance(value, list):
        return [shape(value[0], depth + 1)] if value else []
    return type(value).__name__


def probe(url, method, params, timeout):
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    started = time.time()
    try:
        resp = post(url, body, timeout)
    except (urllib.error.URLError, OSError, ValueError) as e:
        return {"ok": False, "ms": int((time.time() - started) * 1000), "error": str(e)}
    ms = int((time.time() - started) * 1000)
    if "error" in resp:
        return {"ok": False, "ms": ms, "error": resp["error"]}
    return {"ok": True, "ms": ms, "shape": shape(resp.get("result"))}


def measure(name, url, timeout):
    results = {}
    for method, params in PROBES:
        results[method] = probe(url, method, params, timeout)
    ok = [r for r in results.values() if r["ok"]]
    return {
        "provider": name,
        "url": url,
        "ts": int(time.time()),
        "reachable": bool(ok),
        "methods": results,
        "latency_ms": {m: r["ms"] for m, r in results.items()},
        "mean_latency_ms": int(sum(r["ms"] for r in ok) / len(ok)) if ok else None,
    }


def diff(previous, current):
    """Return a list of human-readable drift lines between two measurements."""
    drift = []
    for name, cur in current.items():
        prev = previous.get(name)
        if prev is None:
            continue
        if prev.get("reachable") and not cur.get("reachable"):
            drift.append(f"`{name}`: provider became unreachable")
            continue
        if not prev.get("reachable") or not cur.get("reachable"):
            continue
        for method, _ in PROBES:
            p = prev["methods"].get(method, {})
            c = cur["methods"].get(method, {})
            if p.get("ok") and not c.get("ok"):
                drift.append(f"`{name}`: `{method}` stopped answering: {c.get('error')}")
            elif p.get("ok") and c.get("ok") and p.get("shape") != c.get("shape"):
                drift.append(
                    f"`{name}`: `{method}` result shape changed\n"
                    f"  - before: `{json.dumps(p.get('shape'), sort_keys=True)}`\n"
                    f"  - after:  `{json.dumps(c.get('shape'), sort_keys=True)}`"
                )
    return drift


def load_previous(path):
    if not path or not os.path.exists(path):
        return {}
    with open(path, encoding="utf-8") as f:
        return json.load(f).get("providers", {})


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--provider", action="append", required=True, metavar="NAME=URL")
    p.add_argument("--previous", help="previous measurements.json to diff against")
    p.add_argument("--out", required=True)
    p.add_argument("--timeout", type=float, default=15.0)
    a = p.parse_args(argv)

    providers = {}
    for spec in a.provider:
        name, _, url = spec.partition("=")
        if not url:
            p.error(f"--provider expects NAME=URL, got {spec!r}")
        providers[name] = url

    current = {name: measure(name, url, a.timeout) for name, url in providers.items()}
    drift = diff(load_previous(a.previous), current)

    os.makedirs(a.out, exist_ok=True)
    with open(os.path.join(a.out, "measurements.json"), "w", encoding="utf-8") as f:
        json.dump({"providers": current}, f, indent=2, sort_keys=True)
        f.write("\n")
    with open(os.path.join(a.out, "drift.json"), "w", encoding="utf-8") as f:
        json.dump({"drift": drift}, f, indent=2)
        f.write("\n")
    with open(os.path.join(a.out, "drift.md"), "w", encoding="utf-8") as f:
        if drift:
            f.write("## RPC conformance drift\n\n")
            for line in drift:
                f.write(f"- {line}\n")
        else:
            f.write("No RPC conformance drift detected.\n")

    if not any(m["reachable"] for m in current.values()):
        print("all providers unreachable", file=sys.stderr)
        return 2
    return 1 if drift else 0


if __name__ == "__main__":
    sys.exit(main())
