#!/usr/bin/env python3
"""Generate the public Lafiya transparency dashboard from chain events.

Input is the JSONL event export produced by
`lafiya-watchdog --config <cfg> export --from <L1> --to <L2>` plus a map of
attester address -> region (from `get_attester_info`, see README.md).
Output is a static site: index.html, and for every aggregate a CSV and JSON
file together with the exact ledger range and query that produced it.

Privacy: record counts are only published per (ISO week, state-level region)
cell when the cell holds at least K records; see PRIVACY.md.
"""

import argparse
import csv
import datetime as dt
import html
import json
import os
import sys
from collections import defaultdict

K_ANONYMITY = 10
SUPPRESSED = None  # rendered as "<10" in HTML, empty in CSV, null in JSON
UNKNOWN_REGION = "unknown"


def load_events(path):
    with open(path, encoding="utf-8") as f:
        events = [json.loads(line) for line in f if line.strip()]
    return sorted(events, key=lambda e: (e["ledger"], e.get("tx_hash") or ""))


def week_of(event):
    """ISO week (YYYY-Www) of the ledger close time."""
    closed = event.get("closed_at")
    if not closed:
        raise ValueError(f"event at ledger {event['ledger']} has no closed_at")
    day = dt.datetime.fromisoformat(closed.replace("Z", "+00:00")).date()
    year, week, _ = day.isocalendar()
    return f"{year}-W{week:02d}"


def first_topic(event):
    topics = event.get("topics") or []
    return topics[0] if topics else ""


def verification_activity(events, regions):
    """Counts of new / re-verified / revoked records per (week, region)."""
    seen = set()
    cells = defaultdict(lambda: {"new": 0, "reverified": 0, "revoked": 0})
    record_region = {}
    for e in events:
        name = e["name"]
        if name == "attestation_recorded":
            record = first_topic(e)
            region = regions.get(e.get("data", ""), UNKNOWN_REGION)
            record_region[record] = region
            kind = "reverified" if record in seen else "new"
            seen.add(record)
            cells[(week_of(e), region)][kind] += 1
        elif name == "attestation_revoked":
            record = first_topic(e)
            region = record_region.get(record, UNKNOWN_REGION)
            cells[(week_of(e), region)]["revoked"] += 1
    return cells


def suppress(cells, k=K_ANONYMITY):
    """Apply primary suppression (cells below k) and secondary suppression
    (if a week has exactly one suppressed cell for a metric, also suppress the
    next-smallest one, so it cannot be recovered from the weekly total)."""
    metrics = ("new", "reverified", "revoked")
    out = {key: dict(v) for key, v in cells.items()}
    for key, v in out.items():
        for m in metrics:
            if 0 < v[m] < k:
                v[m] = SUPPRESSED
    weeks = sorted({w for w, _ in out})
    for week in weeks:
        rows = [(r, out[(w, r)]) for (w, r) in out if w == week]
        for m in metrics:
            hidden = [r for r, v in rows if v[m] is SUPPRESSED]
            if len(hidden) == 1:
                visible = sorted(
                    ((v[m], r) for r, v in rows if v[m] not in (SUPPRESSED, 0)),
                )
                if visible:
                    out[(week, visible[0][1])][m] = SUPPRESSED
    return out


def attester_counts(events):
    """Active / suspended / removed attester counts at the end of each week."""
    state = {}
    weekly = {}
    for e in events:
        name = e["name"]
        who = first_topic(e)
        if name == "attester_added":
            state[who] = "active"
        elif name == "attester_suspended":
            state[who] = "suspended"
        elif name == "attester_reinstated":
            state[who] = "active"
        elif name == "attester_removed":
            state[who] = "removed"
        else:
            continue
        counts = {"active": 0, "suspended": 0, "removed": 0}
        for s in state.values():
            counts[s] += 1
        weekly[week_of(e)] = counts
    return [{"week": w, **c} for w, c in sorted(weekly.items())]


def contract_health(events, manifest_hashes):
    """Per contract: current wasm hash vs release manifest, pause state,
    admin, and last admin action, as seen in events."""
    health = {}
    for e in events:
        h = health.setdefault(
            e["contract"],
            {
                "contract": e["contract"],
                "wasm_hash": None,
                "wasm_in_manifest": None,
                "paused": False,
                "admin": None,
                "last_admin_action": None,
            },
        )
        name = e["name"]
        action = {"ledger": e["ledger"], "event": name, "tx_hash": e.get("tx_hash")}
        if name == "upgraded":
            h["wasm_hash"] = first_topic(e).lower()
            h["wasm_in_manifest"] = h["wasm_hash"] in manifest_hashes
            h["last_admin_action"] = action
        elif name == "paused":
            h["paused"] = True
            h["last_admin_action"] = action
        elif name == "unpaused":
            h["paused"] = False
            h["last_admin_action"] = action
        elif name == "initialized":
            h["admin"] = first_topic(e)
            h["last_admin_action"] = action
        elif name == "admin_transferred":
            topics = e.get("topics") or []
            h["admin"] = topics[1] if len(topics) > 1 else None
            h["last_admin_action"] = action
        elif name in (
            "attester_registry_repointed",
            "attester_added",
            "attester_removed",
            "attester_suspended",
            "attester_reinstated",
        ):
            h["last_admin_action"] = action
    return [health[c] for c in sorted(health)]


def manifest_hashes(paths):
    hashes = set()
    for p in paths:
        with open(p, encoding="utf-8") as f:
            for c in json.load(f).get("contracts", []):
                sha = (c.get("wasm") or {}).get("sha256")
                if sha:
                    hashes.add(sha.lower())
    return hashes


def build(events, regions, manifests, provenance):
    activity = suppress(verification_activity(events, regions))
    activity_rows = [
        {"week": w, "region": r, **v} for (w, r), v in sorted(activity.items())
    ]
    return {
        "provenance": provenance,
        "verification_activity": activity_rows,
        "attesters": attester_counts(events),
        "incentives": [],  # populated once M2 incentive contracts exist
        "contract_health": contract_health(events, manifest_hashes(manifests)),
    }


def write_csv(path, rows):
    if not rows:
        open(path, "w", encoding="utf-8").close()
        return
    fields = list(rows[0].keys())
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        w.writeheader()
        for row in rows:
            w.writerow(
                {
                    k: ("" if v is None else json.dumps(v) if isinstance(v, dict) else v)
                    for k, v in row.items()
                }
            )


def cell(v):
    if v is SUPPRESSED:
        return f"&lt;{K_ANONYMITY}"
    if isinstance(v, dict):
        return html.escape(f"{v.get('event')} @ {v.get('ledger')}")
    return html.escape(str(v))


def table(title, key, rows, prov):
    head = f"<h2 id='{key}'>{html.escape(title)}</h2>"
    links = (
        f"<p class='src'>Download: <a href='data/{key}.csv'>CSV</a> · "
        f"<a href='data/{key}.json'>JSON</a> · ledgers {prov['ledger_from']}–"
        f"{prov['ledger_to']} · <a href='data/provenance.json'>query</a></p>"
    )
    if not rows:
        return head + "<p>No data yet.</p>" + links
    cols = list(rows[0].keys())
    th = "".join(f"<th>{html.escape(c)}</th>" for c in cols)
    trs = "".join(
        "<tr>" + "".join(f"<td>{cell(r[c])}</td>" for c in cols) + "</tr>" for r in rows
    )
    return f"{head}<div class='t'><table><tr>{th}</tr>{trs}</table></div>{links}"


def render_html(data):
    prov = data["provenance"]
    sections = [
        table("Verified records per week and region", "verification_activity",
              data["verification_activity"], prov),
        table("Attesters", "attesters", data["attesters"], prov),
        table("Incentive payouts vs verified registrations", "incentives",
              data["incentives"], prov),
        table("Contract health", "contract_health", data["contract_health"], prov),
    ]
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lafiya Transparency</title>
<style>
body{{font:15px/1.5 system-ui,sans-serif;margin:0 auto;max-width:960px;padding:1rem;color:#1b1f23;background:#fff}}
table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #ddd;padding:.3rem .5rem;text-align:left;font-variant-numeric:tabular-nums}}
.t{{overflow-x:auto}}.src{{color:#555;font-size:13px}}
@media (prefers-color-scheme:dark){{body{{background:#111;color:#eee}}td,th{{border-color:#333}}.src{{color:#aaa}}a{{color:#8ab4f8}}}}
</style></head><body>
<h1>Lafiya transparency dashboard</h1>
<p>Every number below is computed from public Soroban contract events on
<b>{html.escape(prov['network'])}</b>, ledgers {prov['ledger_from']}–{prov['ledger_to']}.
Recompute it with the command in <a href="data/provenance.json">provenance.json</a>.
Cells with fewer than {K_ANONYMITY} records are shown as &lt;{K_ANONYMITY}
(<a href="https://github.com/Lafiya-xyz/Lafiya-contract/blob/main/dashboard/PRIVACY.md">privacy review</a>).</p>
{''.join(sections)}
</body></html>
"""


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--events", required=True, help="JSONL from lafiya-watchdog export")
    p.add_argument("--attesters", required=True, help="JSON map attester -> region")
    p.add_argument("--manifest", action="append", default=[], help="release manifest(s)")
    p.add_argument("--network", required=True)
    p.add_argument("--ledger-from", type=int, required=True)
    p.add_argument("--ledger-to", type=int, required=True)
    p.add_argument("--contracts", required=True, help="comma separated contract IDs")
    p.add_argument("--out", required=True)
    args = p.parse_args(argv)

    with open(args.attesters, encoding="utf-8") as f:
        regions = json.load(f)
    events = [
        e for e in load_events(args.events)
        if args.ledger_from <= e["ledger"] < args.ledger_to
    ]
    provenance = {
        "network": args.network,
        "ledger_from": args.ledger_from,
        "ledger_to": args.ledger_to,
        "contracts": args.contracts.split(","),
        "k_anonymity": K_ANONYMITY,
        "query": {
            "method": "getEvents",
            "filters": [{"type": "contract", "contractIds": args.contracts.split(",")}],
            "startLedger": args.ledger_from,
            "endLedger": args.ledger_to,
        },
        "reproduce": (
            f"lafiya-watchdog --config watchdog.toml export --from {args.ledger_from} "
            f"--to {args.ledger_to} > events.jsonl && python3 dashboard/generate.py "
            f"--events events.jsonl --attesters attesters.json --network {args.network} "
            f"--ledger-from {args.ledger_from} --ledger-to {args.ledger_to} "
            f"--contracts {args.contracts} --out site"
        ),
    }
    data = build(events, regions, args.manifest, provenance)

    os.makedirs(os.path.join(args.out, "data"), exist_ok=True)
    for key in ("verification_activity", "attesters", "incentives", "contract_health"):
        write_csv(os.path.join(args.out, "data", f"{key}.csv"), data[key])
        with open(os.path.join(args.out, "data", f"{key}.json"), "w", encoding="utf-8") as f:
            json.dump(data[key], f, indent=2, sort_keys=True)
            f.write("\n")
    with open(os.path.join(args.out, "data", "provenance.json"), "w", encoding="utf-8") as f:
        json.dump(provenance, f, indent=2, sort_keys=True)
        f.write("\n")
    with open(os.path.join(args.out, "index.html"), "w", encoding="utf-8") as f:
        f.write(render_html(data))
    return 0


if __name__ == "__main__":
    sys.exit(main())
