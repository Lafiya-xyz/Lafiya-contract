#!/usr/bin/env python3
"""Protocol-upgrade resource headroom monitor.

Compares per-call measured usage (budgets/baseline.json, refreshed from simulation) with the
current network limits and with the last accepted limits snapshot. Alerts when headroom drops
below a margin or any limit changes; exports Prometheus metrics. Response steps:
docs/runbooks/sdk-protocol-upgrade.md#7-resource-headroom-alerts
"""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
PLAYBOOK = "docs/runbooks/sdk-protocol-upgrade.md"
RESOURCES = (
    "cpu_instructions", "memory_bytes", "ledger_read_entries", "ledger_read_bytes",
    "ledger_write_entries", "ledger_write_bytes", "events_size_bytes",
)


def headroom(baseline: dict, limits: dict) -> list[dict]:
    rows = []
    for contract, fns in baseline.items():
        for fn, usage in fns.items():
            for res in RESOURCES:
                if res in usage and limits.get(res):
                    rows.append({"contract": contract, "function": fn, "resource": res,
                                 "headroom": 1 - usage[res] / limits[res]})
    return rows


def evaluate(baseline: dict, limits: dict, accepted: dict, margin: float) -> tuple[list[dict], list[str]]:
    rows = headroom(baseline, limits)
    alerts = [
        f"{r['contract']}.{r['function']} {r['resource']} headroom {r['headroom']:.1%} < {margin:.0%}"
        for r in rows if r["headroom"] < margin
    ]
    for res in RESOURCES:
        if limits.get(res) != accepted.get(res):
            alerts.append(f"network limit {res} changed: {accepted.get(res)} -> {limits.get(res)}")
    return rows, [f"{a} (playbook: {PLAYBOOK})" for a in alerts]


def metrics(rows: list[dict], limits: dict, alerts: list[str]) -> str:
    out = ["# TYPE lafiya_resource_headroom_ratio gauge"]
    out += [f'lafiya_resource_headroom_ratio{{contract="{r["contract"]}",function="{r["function"]}",'
            f'resource="{r["resource"]}"}} {r["headroom"]:.6f}' for r in rows]
    out.append("# TYPE lafiya_network_limit gauge")
    out += [f'lafiya_network_limit{{resource="{k}"}} {limits[k]}' for k in RESOURCES if k in limits]
    out += ["# TYPE lafiya_headroom_alerts gauge", f"lafiya_headroom_alerts {len(alerts)}"]
    return "\n".join(out) + "\n"


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--baseline", default=ROOT / "budgets" / "baseline.json", type=Path)
    p.add_argument("--limits", required=True, type=Path, help="current network limits JSON")
    p.add_argument("--accepted", default=ROOT / "config" / "network-limits.json", type=Path)
    p.add_argument("--margin", type=float, default=0.2)
    p.add_argument("--metrics-out", type=Path, help="write Prometheus textfile metrics here")
    a = p.parse_args()
    load = lambda f: json.loads(f.read_text(encoding="utf-8"))  # noqa: E731
    limits = load(a.limits)
    rows, alerts = evaluate(load(a.baseline), limits, load(a.accepted), a.margin)
    if a.metrics_out:
        a.metrics_out.write_text(metrics(rows, limits, alerts), encoding="utf-8")
    for alert in alerts:
        print(f"ALERT: {alert}", file=sys.stderr)
    return 1 if alerts else 0


if __name__ == "__main__":
    sys.exit(main())
