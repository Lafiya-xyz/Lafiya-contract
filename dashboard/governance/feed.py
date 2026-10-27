#!/usr/bin/env python3
"""Governance transparency feed: privileged actions from public chain events.

Input: the JSONL event export used by dashboard/generate.py. Output (in --out):
  feed.json   versioned JSON feed (format "lafiya-governance-feed/1")
  feed.atom   the same entries as Atom
  digest.json sha256 of feed.json and the covered ledger range
  digest.sig  Ed25519 signature over digest.json (only with --sign-key)

Only public event fields are used; nothing off-chain is added.
See docs/governance-feed.md for the format and verification instructions.
"""

import argparse
import hashlib
import html
import json
import os
import subprocess
import sys

FORMAT = "lafiya-governance-feed/1"
HERE = os.path.dirname(os.path.abspath(__file__))
PRIVILEGED = {
    "admin_transferred", "upgraded", "paused", "unpaused", "rate_limit_set",
    "attestation_registry_repointed", "attester_added", "attester_removed",
    "attester_suspended", "attester_reinstated", "attester_revoked",
    "attester_info_updated",
}


def load_events(path):
    with open(path, encoding="utf-8") as f:
        events = [json.loads(line) for line in f if line.strip()]
    return sorted(events, key=lambda e: (e["ledger"], e.get("tx_hash") or ""))


def describe(event, locale, catalog):
    table = {**catalog["en"], **catalog.get(locale, {})}
    topics = event.get("topics") or []
    args = {"contract": event["contract"], "topic0": "", "topic1": ""}
    for i, t in enumerate(topics[:2]):
        args[f"topic{i}"] = t
    return table[event["name"]].format(**args)


def build_feed(events, explorer, locale="en", catalog=None):
    catalog = catalog or json.load(open(os.path.join(HERE, "messages.json"), encoding="utf-8"))
    entries = []
    for e in events:
        if e["name"] not in PRIVILEGED:
            continue
        entries.append({
            "id": f"{e['tx_hash']}:{e['name']}:{e['ledger']}",
            "ledger": e["ledger"],
            "closed_at": e.get("closed_at"),
            "contract": e["contract"],
            "event": e["name"],
            "summary": describe(e, locale, catalog),
            "tx_hash": e["tx_hash"],
            "tx_url": explorer.rstrip("/") + "/" + e["tx_hash"],
        })
    ledgers = [e["ledger"] for e in events]
    return {
        "format": FORMAT,
        "locale": locale,
        "ledger_range": [min(ledgers), max(ledgers)] if ledgers else None,
        "entries": entries,
    }


def to_atom(feed):
    updated = feed["entries"][-1]["closed_at"] if feed["entries"] else "1970-01-01T00:00:00Z"
    out = ['<?xml version="1.0" encoding="utf-8"?>',
           '<feed xmlns="http://www.w3.org/2005/Atom">',
           "<title>Lafiya governance transparency feed</title>",
           f"<id>urn:lafiya:governance-feed:{feed['format']}</id>",
           f"<updated>{updated}</updated>"]
    for e in reversed(feed["entries"]):
        out += ["<entry>", f"<id>urn:lafiya:tx:{e['id']}</id>",
                f"<title>{html.escape(e['summary'])}</title>",
                f"<updated>{e['closed_at']}</updated>",
                f'<link href="{html.escape(e["tx_url"])}"/>', "</entry>"]
    out.append("</feed>")
    return "\n".join(out) + "\n"


def canonical(obj):
    return (json.dumps(obj, sort_keys=True, indent=2) + "\n").encode()


def make_digest(feed_bytes, feed):
    return {"format": FORMAT, "feed_sha256": hashlib.sha256(feed_bytes).hexdigest(),
            "ledger_range": feed["ledger_range"], "entries": len(feed["entries"])}


def verify(events, feed_path, digest_path):
    """Recompute the feed from chain events and compare with the published files."""
    feed = json.load(open(feed_path, encoding="utf-8"))
    expected = build_feed(events, feed["entries"][0]["tx_url"].rsplit("/", 1)[0] if feed["entries"] else "", feed["locale"])
    if expected != feed:
        return "feed does not match chain events"
    digest = json.load(open(digest_path, encoding="utf-8"))
    if digest != make_digest(open(feed_path, "rb").read(), feed):
        return "digest does not match feed.json"
    return None


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--events", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--explorer", default="https://stellar.expert/explorer/public/tx")
    p.add_argument("--locale", default="en")
    p.add_argument("--sign-key", help="Ed25519 private key (PEM) used to sign digest.json")
    p.add_argument("--verify", action="store_true", help="verify existing --out files against --events")
    a = p.parse_args(argv)
    events = load_events(a.events)
    if a.verify:
        err = verify(events, os.path.join(a.out, "feed.json"), os.path.join(a.out, "digest.json"))
        if err:
            print(f"FAIL: {err}", file=sys.stderr)
            return 1
        print("OK: feed matches chain events")
        return 0
    feed = build_feed(events, a.explorer, a.locale)
    os.makedirs(a.out, exist_ok=True)
    data = canonical(feed)
    write = lambda n, b: open(os.path.join(a.out, n), "wb").write(b)
    write("feed.json", data)
    write("feed.atom", to_atom(feed).encode())
    write("digest.json", canonical(make_digest(data, feed)))
    if a.sign_key:
        subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", a.sign_key,
                        "-in", os.path.join(a.out, "digest.json"),
                        "-out", os.path.join(a.out, "digest.sig")], check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
