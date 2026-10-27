#!/usr/bin/env python3
"""Fault-injecting Soroban RPC proxy and scenario runner (stdlib only).

  chaos.py run sandbox/chaos/scenarios.toml        # run every scenario
  chaos.py proxy --upstream URL --fault stale_tip  # standalone proxy

The proxy forwards JSON-RPC calls to --upstream and applies one fault per
scenario. Each scenario starts the proxy, runs `command` with `{rpc_url}`
substituted, and asserts on its exit code and output. Without --upstream the
runner uses a built-in stub RPC, so it needs no network. See README.md.
"""

import argparse
import http.server
import json
import subprocess
import sys
import threading
import time
import tomllib
import urllib.request

LATEST = 5000


def stub_result(method, params):
    if method == "getLatestLedger":
        return {"id": "stub", "protocolVersion": 23, "sequence": LATEST}
    if method == "getEvents":
        start = (params or {}).get("startLedger", LATEST - 3)
        return {"events": [{"ledger": l, "id": f"{l:010d}-0"} for l in range(start, LATEST + 1)],
                "latestLedger": LATEST}
    if method == "getHealth":
        return {"status": "healthy", "latestLedger": LATEST}
    return {}


# --- faults -----------------------------------------------------------------
# Each fault takes (request_json, upstream_response_bytes, opts) and returns
# (status, body_bytes), or raises ConnectionAbortedError to drop the socket.
# To add one: write a function, register it in FAULTS, add a scenario.

def latency(req, body, o):
    time.sleep(o.get("delay_ms", 1500) / 1000)
    return 200, body


def truncated_body(req, body, o):
    return 200, body[: max(1, len(body) // 2)]


def malformed_json(req, body, o):
    return 200, b"<html>502 Bad Gateway</html>"


def http_500(req, body, o):
    return 500, b'{"error":"internal"}'


def rate_limited(req, body, o):
    return 429, b'{"error":"rate limited"}'


def connection_drop(req, body, o):
    raise ConnectionAbortedError


def empty_result(req, body, o):
    return 200, json.dumps({"jsonrpc": "2.0", "id": req.get("id"), "result": {}}).encode()


def stale_tip(req, body, o):
    """getLatestLedger / latestLedger lag the real tip by `lag` ledgers."""
    doc = json.loads(body)
    r = doc.get("result") or {}
    for key in ("sequence", "latestLedger"):
        if key in r:
            r[key] -= o.get("lag", 100)
    return 200, json.dumps(doc).encode()


def ledger_gap(req, body, o):
    """getEvents silently omits every `every`-th ledger."""
    doc = json.loads(body)
    evs = (doc.get("result") or {}).get("events")
    if evs is not None:
        doc["result"]["events"] = [e for e in evs if e["ledger"] % o.get("every", 2)]
    return 200, json.dumps(doc).encode()


def clock_skew(req, body, o):
    """Shift every ledger close time field by `skew_s` seconds."""
    doc = json.loads(body)

    def shift(x):
        if isinstance(x, dict):
            for k, v in x.items():
                if k in ("ledgerClosedAt", "closedAt") and isinstance(v, str):
                    x[k] = f"skewed+{o.get('skew_s', 3600)}s:{v}"
                else:
                    shift(v)
        elif isinstance(x, list):
            for i in x:
                shift(i)
    shift(doc)
    return 200, json.dumps(doc).encode()


def lying_rpc(req, body, o):
    """Reports success with a fabricated, plausible-looking result."""
    return 200, json.dumps({"jsonrpc": "2.0", "id": req.get("id"),
                            "result": {"status": "SUCCESS", "hash": "00" * 32}}).encode()


FAULTS = {f.__name__: f for f in (
    latency, truncated_body, malformed_json, http_500, rate_limited,
    connection_drop, empty_result, stale_tip, ledger_gap, clock_skew, lying_rpc)}


class Proxy(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, upstream, fault, opts=None):
        self.upstream, self.opts = upstream, opts or {}
        self.fault = FAULTS[fault] if fault else None
        super().__init__(("127.0.0.1", 0), Handler)

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_address[1]}"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        req = json.loads(raw or b"{}")
        if self.server.upstream:
            r = urllib.request.Request(self.server.upstream, raw,
                                       {"Content-Type": "application/json"})
            body = urllib.request.urlopen(r, timeout=30).read()
        else:
            body = json.dumps({"jsonrpc": "2.0", "id": req.get("id"),
                               "result": stub_result(req.get("method"), req.get("params"))}).encode()
        status = 200
        try:
            if self.server.fault:
                status, body = self.server.fault(req, body, self.server.opts)
        except ConnectionAbortedError:
            self.connection.close()
            return
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run_scenario(sc, upstream):
    proxy = Proxy(upstream, sc.get("fault"), sc.get("options"))
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    try:
        cmd = [a.replace("{rpc_url}", proxy.url) for a in sc["command"]]
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=sc.get("timeout_s", 30))
        out = p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        p, out = None, ""
    finally:
        proxy.shutdown()
    errs = []
    if p is None:
        errs.append("command timed out (tooling hung under fault)")
    else:
        if "expect_exit" in sc and p.returncode != sc["expect_exit"]:
            errs.append(f"exit {p.returncode}, expected {sc['expect_exit']}")
        for needle in sc.get("expect_output", []):
            if needle not in out:
                errs.append(f"output missing {needle!r}")
    return errs, out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("scenarios")
    r.add_argument("--upstream")
    x = sub.add_parser("proxy")
    x.add_argument("--upstream")
    x.add_argument("--fault", choices=sorted(FAULTS))
    args = ap.parse_args(argv)
    if args.cmd == "proxy":
        p = Proxy(args.upstream, args.fault)
        print(p.url, flush=True)
        p.serve_forever()
    with open(args.scenarios, "rb") as f:
        scenarios = tomllib.load(f)["scenario"]
    failed = 0
    for sc in scenarios:
        errs, out = run_scenario(sc, args.upstream)
        print(("FAIL " if errs else "ok   ") + sc["name"])
        for e in errs:
            print("     -", e)
        failed += bool(errs)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
