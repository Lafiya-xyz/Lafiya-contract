#!/usr/bin/env python3
"""Generate the static status page from probe history + incident annotations.

  status_page.py --history h1.jsonl [h2.jsonl ...] --incidents incidents.json \
      --out site/status [--retention-days 90]

A probe counts as down only when it fails from every location in the same
time bucket (5 min), which suppresses single-vantage false positives.
`incidents.json`: [{"start": ts, "end": ts|null, "title": "...", "note": "..."}].
"""

import argparse
import html
import json
import os
import sys
import time

BUCKET = 300


def load(paths, cutoff):
    recs = []
    for p in paths:
        with open(p, encoding="utf-8") as f:
            recs += [r for r in map(json.loads, filter(str.strip, f)) if r["ts"] >= cutoff]
    return recs


def summarize(recs):
    """{(deployment, probe): {bucket: ok}} where ok = any location succeeded."""
    cells = {}
    for r in recs:
        for pr in r["probes"]:
            b = r["ts"] // BUCKET
            c = cells.setdefault((r["deployment"], pr["probe"]), {})
            c[b] = c.get(b, False) or pr["ok"]
    return cells


def render(cells, incidents, now):
    rows = []
    for (dep, probe), buckets in sorted(cells.items()):
        last = buckets[max(buckets)]
        up = 100.0 * sum(buckets.values()) / len(buckets)
        rows.append(f"<tr><td>{html.escape(dep)}</td><td>{html.escape(probe)}</td>"
                    f"<td>{'up' if last else 'DOWN'}</td><td>{up:.2f}%</td></tr>")
    inc = "".join(
        f"<li><b>{html.escape(i['title'])}</b> ({i['start']}–{i.get('end') or 'ongoing'}): {html.escape(i.get('note', ''))}</li>"
        for i in sorted(incidents, key=lambda i: -i["start"]))
    return ("<!doctype html><meta charset=utf-8><title>Lafiya status</title>"
            f"<h1>Lafiya status</h1><p>Generated {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(now))}</p>"
            "<table><tr><th>Deployment</th><th>Probe</th><th>Now</th><th>Uptime</th></tr>"
            + "".join(rows) + f"</table><h2>Incidents</h2><ul>{inc}</ul>")


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--history", nargs="+", required=True)
    p.add_argument("--incidents", default=None)
    p.add_argument("--out", required=True)
    p.add_argument("--retention-days", type=int, default=90)
    p.add_argument("--now", type=int, default=None)
    a = p.parse_args(argv)
    now = a.now or int(time.time())
    recs = load(a.history, now - a.retention_days * 86400)
    incidents = json.load(open(a.incidents)) if a.incidents and os.path.exists(a.incidents) else []
    cells = summarize(recs)
    os.makedirs(a.out, exist_ok=True)
    open(os.path.join(a.out, "index.html"), "w", encoding="utf-8").write(render(cells, incidents, now))
    json.dump({f"{d}/{p_}": {"buckets": len(b), "up": sum(b.values())} for (d, p_), b in cells.items()},
              open(os.path.join(a.out, "status.json"), "w"), indent=2, sort_keys=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
