#!/usr/bin/env python3
"""Per-commit benchmark history, drift alerts, and the trend page.

`budgets/baseline.json` only holds the latest allowed state, so a footprint
that grows 1% per PR never trips it. This script keeps one metrics file per
commit on `main` (the `bench-history` data branch, see
.github/workflows/bench-history.yml). From that history it renders a trend
page and raises alerts when a metric drifts and stays drifted.

Metrics come from `lafiya-bench` (crates/lafiya-bench), run by
scripts/bench_measure.sh on a checkout of the commit being measured.

Usage:
    scripts/bench_history.py record   --history DIR --commit SHA --metrics FILE
    scripts/bench_history.py backfill --history DIR [--count 50] [--ref main]
    scripts/bench_history.py alerts   --history DIR [--json PATH] [--markdown PATH]
    scripts/bench_history.py render   --history DIR --out PATH

Alerting (budgets/trend.json) uses hysteresis to keep noise down. A metric
raises once it is more than `tolerance` above the median of its previous
`lookback` points for `sustain` consecutive commits. One noisy commit does
not raise. A raised alert holds its reference value and clears only after
`sustain` consecutive commits back within `clear_tolerance` of it. An
intended increase is accepted by adding `"<metric key>": "<sha>"` to
`acknowledged`, which rebaselines that metric from the commit onward.
"""

import argparse
import json
import os
import shlex
import statistics
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "budgets" / "trend.json"
DEFAULT_MEASURE = str(ROOT / "scripts" / "bench_measure.sh")

Json = dict[str, Any]


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def measure(repo: Path, commit: str, cmd: str) -> Json | None:
    """Run `cmd` in a detached worktree of `commit`; None if it fails.

    `cmd` must print the lafiya-bench JSON on stdout. A commit that does not
    build is a skip, not an error, for both backfill and bisect.
    """
    with tempfile.TemporaryDirectory(prefix="bench-") as tmp:
        tree = Path(tmp) / "tree"
        git(repo, "worktree", "add", "--quiet", "--detach", str(tree), commit)
        try:
            proc = subprocess.run(
                shlex.split(cmd),
                cwd=tree,
                capture_output=True,
                text=True,
                env={**os.environ, "BENCH_COMMIT": commit},
            )
        finally:
            git(repo, "worktree", "remove", "--force", str(tree))
    if proc.returncode != 0:
        sys.stderr.write(f"{commit[:12]}: measure failed\n{proc.stderr[-2000:]}\n")
        return None
    try:
        result: Json = json.loads(proc.stdout)
    except json.JSONDecodeError:
        sys.stderr.write(f"{commit[:12]}: measure printed no JSON\n")
        return None
    if not result.get("metrics"):
        sys.stderr.write(f"{commit[:12]}: no contract built\n")
        return None
    return result


def record(history: Path, repo: Path, commit: str, result: Json) -> Path:
    sha = git(repo, "rev-parse", commit)
    entry = {
        "commit": sha,
        # First-parent depth orders main's history even when backfilled late.
        "position": int(git(repo, "rev-list", "--count", "--first-parent", sha)),
        "date": git(repo, "show", "-s", "--format=%cI", sha),
        "subject": git(repo, "show", "-s", "--format=%s", sha),
        "metrics": result.get("metrics", {}),
        "errors": result.get("errors", {}),
    }
    path = history / "commits" / f"{sha}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(entry, indent=2, sort_keys=True) + "\n")
    return path


def load_history(history: Path) -> list[Json]:
    entries = [json.loads(p.read_text()) for p in (history / "commits").glob("*.json")]
    return sorted(entries, key=lambda e: int(e["position"]))


@dataclass
class Alert:
    metric: str
    since: str
    reference: float
    current: float
    commit: str

    @property
    def change(self) -> float:
        return ratio(self.current, self.reference)


@dataclass
class Event:
    metric: str
    kind: str  # "raise" | "clear"
    commit: str


def ratio(value: float, reference: float) -> float:
    if reference == 0:
        return 0.0 if value == 0 else float("inf")
    return value / reference - 1


def series(entries: list[Json], key: str, acked: str | None) -> list[tuple[str, float]]:
    points: list[tuple[str, float]] = []
    for e in entries:
        if acked and e["commit"].startswith(acked):
            points = []
        if key in e["metrics"]:
            points.append((e["commit"], float(e["metrics"][key])))
    return points


def evaluate(
    points: list[tuple[str, float]], key: str, cfg: Json
) -> tuple[Alert | None, list[Event]]:
    tol, clear, sustain = cfg["tolerance"], cfg["clear_tolerance"], cfg["sustain"]
    lookback = cfg["lookback"]
    alert: Alert | None = None
    events: list[Event] = []
    run = 0
    for i, (commit, value) in enumerate(points):
        if alert is None:
            prior = [v for _, v in points[max(0, i - run - lookback) : i - run]]
            if len(prior) < sustain:
                continue
            reference = statistics.median(prior)
            run = run + 1 if ratio(value, reference) > tol else 0
            if run >= sustain:
                since = points[i - sustain + 1][0]
                alert = Alert(key, since, reference, value, commit)
                events.append(Event(key, "raise", since))
                run = 0
        else:
            alert.current, alert.commit = value, commit
            run = run + 1 if ratio(value, alert.reference) <= clear else 0
            if run >= sustain:
                events.append(Event(key, "clear", commit))
                alert, run = None, 0
    return alert, events


def metric_keys(entries: list[Json], cfg: Json) -> list[str]:
    wanted = set(cfg["metrics"])
    keys = {k for e in entries for k in e["metrics"] if k.rsplit("/", 1)[-1] in wanted}
    return sorted(keys)


def all_alerts(entries: list[Json], cfg: Json) -> tuple[list[Alert], list[Event]]:
    active: list[Alert] = []
    events: list[Event] = []
    for key in metric_keys(entries, cfg):
        alert, evs = evaluate(series(entries, key, cfg["acknowledged"].get(key)), key, cfg)
        events += evs
        if alert:
            active.append(alert)
    return active, events


def alerts_markdown(active: list[Alert]) -> str:
    if not active:
        return "No sustained benchmark drift.\n"
    lines = [
        "Sustained resource-usage drift on `main` (see docs/benchmarks.md).",
        "",
        "| Metric | Drifting since | Reference | Latest | Change |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for a in active:
        lines.append(
            f"| `{a.metric}` | {a.since[:12]} | {a.reference:,.0f} | {a.current:,.0f} "
            f"| +{a.change:.1%} |"
        )
    lines += [
        "",
        "Find the commit with `scripts/bench_bisect.py --good <since>^ --bad "
        "<latest> --metric <metric>`. If the increase is intended, acknowledge it "
        "in `budgets/trend.json`.",
    ]
    return "\n".join(lines) + "\n"


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lafiya resource-usage trends</title>
<style>
:root { --surface: #fcfcfb; --ink: #0b0b0b; --ink-2: #52514e; --grid: #e4e3df;
  --series: #2a78d6; --critical: #d03b3b; color-scheme: light; }
@media (prefers-color-scheme: dark) {
  :root { --surface: #1a1a19; --ink: #f0efec; --ink-2: #c3c2b7; --grid: #383835;
    --series: #3987e5; color-scheme: dark; }
}
body { margin: 0; padding: 24px 16px; background: var(--surface); color: var(--ink);
  font: 14px/1.4 system-ui, sans-serif; }
main { max-width: 1200px; margin: 0 auto; }
h1 { font-size: 20px; margin: 0 0 4px; }
p.meta, .card small { color: var(--ink-2); }
.controls { display: flex; gap: 12px; align-items: center; margin: 16px 0; flex-wrap: wrap; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 16px; }
.card { border: 1px solid var(--grid); border-radius: 8px; padding: 12px; }
.card h2 { font-size: 14px; margin: 0 0 2px; font-weight: 600; }
.badge { color: var(--critical); font-weight: 600; }
svg { width: 100%; height: 140px; display: block; }
svg .axis { stroke: var(--grid); }
svg .line { fill: none; stroke: var(--series); stroke-width: 2; }
svg .dot { fill: var(--series); stroke: var(--surface); stroke-width: 2; }
svg text { fill: var(--ink-2); font-size: 11px; }
.tip { position: fixed; pointer-events: none; background: var(--surface); color: var(--ink);
  border: 1px solid var(--grid); border-radius: 6px; padding: 6px 8px; font-size: 12px; }
.table-wrap { overflow-x: auto; }
table { border-collapse: collapse; font-size: 12px; }
td, th { padding: 2px 8px; text-align: right; border-bottom: 1px solid var(--grid); }
th:first-child, td:first-child { text-align: left; }
</style>
</head>
<body>
<main>
<h1>Contract resource usage on main</h1>
<p class="meta">__SUMMARY__</p>
<div class="controls">
  <label>Metric <select id="metric"></select></label>
</div>
<div class="grid" id="grid"></div>
<details><summary>Table view</summary><div class="table-wrap" id="table"></div></details>
</main>
<div class="tip" id="tip" hidden></div>
<script>
const DATA = __DATA__;
const select = document.getElementById("metric");
const metrics = [...new Set(DATA.keys.map(k => k.split("/").pop()))];
for (const m of metrics) select.add(new Option(m, m));
if (metrics.includes("cpu_instructions")) select.value = "cpu_instructions";
const tip = document.getElementById("tip");
const fmt = v => v.toLocaleString("en-US");

function chart(key) {
  const pts = DATA.commits.map((c, i) => [i, c.metrics[key]]).filter(p => p[1] !== undefined);
  const card = document.createElement("div");
  card.className = "card";
  const alert = DATA.alerts.find(a => a.metric === key);
  const name = key.split("/").slice(0, -1).join(" / ");
  const latest = pts.length ? fmt(pts[pts.length - 1][1]) : "no data";
  const badge = alert
    ? ` <span class="badge">&#9888; drift +${(alert.change * 100).toFixed(1)}%</span>`
    : "";
  card.innerHTML = `<h2>${name}</h2><small>${latest}${badge}</small>`;
  if (!pts.length) return card;
  const W = 340, H = 140, L = 4, R = 4, T = 10, B = 18;
  const vals = pts.map(p => p[1]);
  let lo = Math.min(...vals), hi = Math.max(...vals);
  if (lo === hi) { lo = lo * 0.95; hi = hi * 1.05 || 1; }
  const n = Math.max(DATA.commits.length - 1, 1);
  const x = i => L + (W - L - R) * i / n;
  const y = v => T + (H - T - B) * (1 - (v - lo) / (hi - lo));
  const d = pts
    .map((p, j) => `${j ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`)
    .join("");
  const last = pts[pts.length - 1];
  const tail = DATA.commits[n] ? DATA.commits[n].commit.slice(0, 7) : "";
  const svg = `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img"
    aria-label="${name} ${select.value}">
    <line class="axis" x1="${L}" x2="${W - R}" y1="${H - B}" y2="${H - B}"/>
    <path class="line" d="${d}"/>
    <circle class="dot" r="4" cx="${x(last[0])}" cy="${y(last[1])}"/>
    <text x="${L}" y="${H - 4}">${DATA.commits[0].commit.slice(0, 7)}</text>
    <text x="${W - R}" y="${H - 4}" text-anchor="end">${tail}</text>
  </svg>`;
  card.insertAdjacentHTML("beforeend", svg);
  const el = card.querySelector("svg");
  el.addEventListener("mousemove", ev => {
    const r = el.getBoundingClientRect();
    const i = Math.round((ev.clientX - r.left) / r.width * n);
    const p = pts.reduce((a, b) => Math.abs(b[0] - i) < Math.abs(a[0] - i) ? b : a);
    const c = DATA.commits[p[0]];
    tip.textContent = `${c.commit.slice(0, 7)} · ${fmt(p[1])} · ${c.subject}`;
    tip.style.left = `${ev.clientX + 12}px`;
    tip.style.top = `${ev.clientY + 12}px`;
    tip.hidden = false;
  });
  el.addEventListener("mouseleave", () => { tip.hidden = true; });
  return card;
}

function render() {
  const m = select.value;
  const keys = DATA.keys.filter(k => k.endsWith("/" + m));
  const grid = document.getElementById("grid");
  grid.replaceChildren(...keys.map(chart));
  const cols = keys.map(k => `<th>${k.split("/").slice(0, -1).join("/")}</th>`);
  const head = `<tr><th>commit</th>${cols.join("")}</tr>`;
  const cell = (c, k) => `<td>${c.metrics[k] === undefined ? "" : fmt(c.metrics[k])}</td>`;
  const rows = DATA.commits.map(c => `<tr><td>${c.commit.slice(0, 7)}</td>`
    + keys.map(k => cell(c, k)).join("") + "</tr>");
  document.getElementById("table").innerHTML = `<table>${head}${rows.join("")}</table>`;
}
select.addEventListener("change", render);
render();
</script>
</body>
</html>
"""


def render_page(entries: list[Json], cfg: Json) -> str:
    active, _ = all_alerts(entries, cfg)
    data = {
        "keys": metric_keys(entries, cfg),
        "commits": [
            {"commit": e["commit"], "subject": e["subject"], "metrics": e["metrics"]}
            for e in entries
        ],
        "alerts": [{**asdict(a), "change": a.change} for a in active],
    }
    summary = f"{len(entries)} commits"
    if entries:
        summary += f", {entries[0]['commit'][:7]}..{entries[-1]['commit'][:7]}"
    summary += f"; {len(active)} active drift alert(s)."
    blob = json.dumps(data, separators=(",", ":")).replace("</", "<\\/")
    return PAGE.replace("__SUMMARY__", summary).replace("__DATA__", blob)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    sub = parser.add_subparsers(dest="command", required=True)

    rec = sub.add_parser("record", help="store one commit's metrics")
    rec.add_argument("--history", type=Path, required=True)
    rec.add_argument("--commit", default="HEAD")
    rec.add_argument("--metrics", type=Path, required=True)

    back = sub.add_parser("backfill", help="measure first-parent commits missing from history")
    back.add_argument("--history", type=Path, required=True)
    back.add_argument("--count", type=int, default=50)
    back.add_argument("--ref", default="HEAD")
    back.add_argument("--measure-cmd", default=DEFAULT_MEASURE)

    al = sub.add_parser("alerts", help="evaluate drift alerts")
    al.add_argument("--history", type=Path, required=True)
    al.add_argument("--json", type=Path)
    al.add_argument("--markdown", type=Path)

    ren = sub.add_parser("render", help="write the trend page")
    ren.add_argument("--history", type=Path, required=True)
    ren.add_argument("--out", type=Path, required=True)

    args = parser.parse_args(argv)
    cfg: Json = json.loads(args.config.read_text())

    if args.command == "record":
        print(record(args.history, args.repo, args.commit, json.loads(args.metrics.read_text())))
    elif args.command == "backfill":
        commits = git(
            args.repo, "rev-list", "--first-parent", f"--max-count={args.count}", args.ref
        ).split()
        done = {e["commit"] for e in load_history(args.history)} if args.history.exists() else set()
        skipped = 0
        for commit in reversed(commits):
            if commit in done:
                continue
            result = measure(args.repo, commit, args.measure_cmd)
            if result is None:
                skipped += 1
                continue
            print(record(args.history, args.repo, commit, result))
        print(f"backfill: {len(commits)} commits, {skipped} skipped (did not build)")
    elif args.command == "alerts":
        active, events = all_alerts(load_history(args.history), cfg)
        if args.json:
            args.json.write_text(
                json.dumps(
                    {
                        "active": [{**asdict(a), "change": a.change} for a in active],
                        "events": [asdict(e) for e in events],
                    },
                    indent=2,
                )
                + "\n"
            )
        if args.markdown:
            args.markdown.write_text(alerts_markdown(active))
        sys.stdout.write(alerts_markdown(active))
    elif args.command == "render":
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render_page(load_history(args.history), cfg))
        print(args.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
