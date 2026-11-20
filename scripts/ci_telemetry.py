#!/usr/bin/env python3
"""CI workflow cost and duration telemetry with week-over-week regression alerts.

Collects per-job duration and billable minutes for the `CI` workflow from the
GitHub API into a JSONL history (one line per job run), renders a static
report, and flags a regression when the p50 PR pipeline duration of the last
complete week is more than `threshold` above the previous week.

Usage:
    scripts/ci_telemetry.py collect --history DIR [--workflow ci.yml] [--days 14]
    scripts/ci_telemetry.py report  --history DIR --out PATH [--json PATH]
    scripts/ci_telemetry.py alerts  --history DIR [--json PATH] [--markdown PATH]

`collect` needs `GH_TOKEN` (or `GITHUB_TOKEN`) and `GITHUB_REPOSITORY`; it
shells out to `gh api`, which is already authenticated in Actions.
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "budgets" / "ci-telemetry.json"
HISTORY_FILE = "jobs.jsonl"

Json = dict[str, Any]


def gh_api(path: str, *params: str) -> Any:
    """GET a GitHub API path with `gh api`, following pagination."""
    cmd = ["gh", "api", "--paginate", path, *params]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise SystemExit(f"gh api {path} failed:\n{proc.stderr[-2000:]}")
    # --paginate concatenates JSON arrays; merge them.
    out: list[Any] = []
    decoder = json.JSONDecoder()
    text, idx = proc.stdout, 0
    while idx < len(text):
        while idx < len(text) and text[idx].isspace():
            idx += 1
        if idx >= len(text):
            break
        value, idx = decoder.raw_decode(text, idx)
        out.extend(value if isinstance(value, list) else [value])
    return out


def iso(ts: str) -> datetime:
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


def collect(history: Path, repo: str, workflow: str, days: int) -> int:
    """Append job rows for runs of `workflow` started in the last `days`."""
    since = datetime.now(timezone.utc) - timedelta(days=days)
    runs = gh_api(
        f"repos/{repo}/actions/workflows/{workflow}/runs",
        "-f", "per_page=100",
        "-f", "status=completed",
    )
    path = history / HISTORY_FILE
    path.parent.mkdir(parents=True, exist_ok=True)
    seen: set[str] = set()
    if path.exists():
        for line in path.read_text().splitlines():
            if line.strip():
                seen.add(json.loads(line)["job_id"])

    added = 0
    with path.open("a") as fh:
        for run in runs:
            if iso(run["created_at"]) < since:
                continue
            jobs = gh_api(f"repos/{repo}/actions/runs/{run['id']}/jobs", "-f", "per_page=100")
            for job in jobs:
                if job["id"] in seen or not job.get("completed_at"):
                    continue
                row = {
                    "job_id": job["id"],
                    "run_id": run["id"],
                    "workflow": run.get("name", workflow),
                    "event": run.get("event"),
                    "head_branch": run.get("head_branch"),
                    "conclusion": job.get("conclusion"),
                    "started_at": job["started_at"],
                    "completed_at": job["completed_at"],
                    "duration_s": round(
                        (iso(job["completed_at"]) - iso(job["started_at"])).total_seconds(), 1
                    ),
                    "runner": (job.get("labels") or ["unknown"])[0],
                }
                fh.write(json.dumps(row, sort_keys=True) + "\n")
                seen.add(job["id"])
                added += 1
    print(f"collect: {added} new job row(s) in {path}")
    return added


def load(history: Path) -> list[Json]:
    path = history / HISTORY_FILE
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def week_start(ts: str) -> str:
    d = iso(ts).date()
    return (d - timedelta(days=d.weekday())).isoformat()


def pipeline_durations(rows: list[Json]) -> dict[str, list[float]]:
    """Per ISO week, the wall-clock duration of each PR pipeline run."""
    runs: dict[int, Json] = {}
    for row in rows:
        if row.get("event") != "pull_request":
            continue
        run = runs.setdefault(row["run_id"], {"start": row["started_at"], "end": row["completed_at"]})
        run["start"] = min(run["start"], row["started_at"])
        run["end"] = max(run["end"], row["completed_at"])
    weeks: dict[str, list[float]] = {}
    for run in runs.values():
        weeks.setdefault(week_start(run["start"]), []).append(
            (iso(run["end"]) - iso(run["start"])).total_seconds()
        )
    return weeks


def evaluate(rows: list[Json], cfg: Json) -> Json:
    weeks = pipeline_durations(rows)
    ordered = sorted(weeks)
    result: Json = {"weeks": [], "regression": False, "threshold": cfg["threshold"]}
    if len(ordered) < 2:
        return result
    current, previous = ordered[-1], ordered[-2]
    p50_now = statistics.median(weeks[current])
    p50_prev = statistics.median(weeks[previous])
    change = (p50_now / p50_prev - 1) if p50_prev else 0.0
    result.update(
        {
            "current_week": current,
            "previous_week": previous,
            "p50_current_s": round(p50_now, 1),
            "p50_previous_s": round(p50_prev, 1),
            "change": round(change, 4),
            "regression": change > cfg["threshold"],
        }
    )
    for week in ordered:
        result["weeks"].append(
            {
                "week": week,
                "runs": len(weeks[week]),
                "p50_s": round(statistics.median(weeks[week]), 1),
                "total_minutes": round(sum(weeks[week]) / 60, 1),
            }
        )
    return result


def job_table(rows: list[Json]) -> list[str]:
    by_job: dict[str, list[float]] = {}
    for row in rows:
        by_job.setdefault(row.get("runner", "unknown"), []).append(row["duration_s"])
    lines = ["| Runner | Jobs | p50 duration (s) | Total minutes |", "| --- | ---: | ---: | ---: |"]
    for runner, values in sorted(by_job.items()):
        lines.append(
            f"| `{runner}` | {len(values)} | {statistics.median(values):,.1f} "
            f"| {sum(values) / 60:,.1f} |"
        )
    return lines


def alerts_markdown(rows: list[Json], result: Json) -> str:
    lines = ["## CI pipeline duration regression", ""]
    if result.get("regression"):
        lines += [
            f"p50 PR pipeline time for the week of {result['current_week']} is "
            f"**{result['p50_current_s']:,.0f}s**, {result['change']:+.1%} versus "
            f"{result['p50_previous_s']:,.0f}s the week before "
            f"(threshold {result['threshold']:.0%}).",
            "",
        ]
    else:
        lines += ["No week-over-week regression above the threshold.", ""]
    lines += ["| Week | PR runs | p50 (s) | Total minutes |", "| --- | ---: | ---: | ---: |"]
    for week in result.get("weeks", []):
        lines.append(
            f"| {week['week']} | {week['runs']} | {week['p50_s']:,.1f} | {week['total_minutes']:,.1f} |"
        )
    lines += ["", "### Per-job cost", ""] + job_table(rows)
    return "\n".join(lines) + "\n"


def render_page(rows: list[Json], result: Json) -> str:
    blob = json.dumps(result, separators=(",", ":")).replace("</", "<\\/")
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lafiya CI cost and duration</title>
<style>
body {{ margin: 0; padding: 24px 16px; font: 14px/1.4 system-ui, sans-serif; }}
main {{ max-width: 960px; margin: 0 auto; }}
table {{ border-collapse: collapse; width: 100%; margin: 12px 0; }}
th, td {{ border-bottom: 1px solid #e4e3df; padding: 6px 8px; text-align: right; }}
th:first-child, td:first-child {{ text-align: left; }}
.bad {{ color: #d03b3b; font-weight: 600; }}
</style>
</head>
<body>
<main>
<h1>Lafiya CI cost and duration</h1>
<p id="summary"></p>
<table id="weeks"><thead><tr><th>Week</th><th>PR runs</th><th>p50 (s)</th><th>Total minutes</th></tr></thead><tbody></tbody></table>
<table id="jobs"><thead><tr><th>Runner</th><th>Jobs</th><th>p50 duration (s)</th><th>Total minutes</th></tr></thead><tbody></tbody></table>
</main>
<script>
const data = {blob};
const summary = document.getElementById("summary");
summary.textContent = data.regression
  ? `Regression: p50 PR pipeline time ${{(data.change * 100).toFixed(1)}}% above the previous week.`
  : "No week-over-week regression above the threshold.";
if (data.regression) summary.className = "bad";
const weeks = document.querySelector("#weeks tbody");
for (const w of data.weeks || []) {{
  const tr = document.createElement("tr");
  for (const v of [w.week, w.runs, w.p50_s.toFixed(1), w.total_minutes.toFixed(1)]) {{
    const td = document.createElement("td");
    td.textContent = v;
    tr.appendChild(td);
  }}
  weeks.appendChild(tr);
}}
</script>
</body>
</html>
"""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    sub = parser.add_subparsers(dest="command", required=True)

    col = sub.add_parser("collect", help="append job rows from the GitHub API")
    col.add_argument("--history", type=Path, required=True)
    col.add_argument("--workflow", default="ci.yml")
    col.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    col.add_argument("--days", type=int, default=14)

    rep = sub.add_parser("report", help="render the static report")
    rep.add_argument("--history", type=Path, required=True)
    rep.add_argument("--out", type=Path, required=True)
    rep.add_argument("--json", type=Path)

    al = sub.add_parser("alerts", help="evaluate the week-over-week regression")
    al.add_argument("--history", type=Path, required=True)
    al.add_argument("--json", type=Path)
    al.add_argument("--markdown", type=Path)

    args = parser.parse_args(argv)
    cfg: Json = json.loads(args.config.read_text())

    if args.command == "collect":
        if not args.repo:
            raise SystemExit("collect: --repo or GITHUB_REPOSITORY is required")
        collect(args.history, args.repo, args.workflow, args.days)
        return 0

    rows = load(args.history)
    result = evaluate(rows, cfg)
    if args.command == "report":
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render_page(rows, result))
        if args.json:
            args.json.write_text(json.dumps(result, indent=2) + "\n")
    else:
        if args.json:
            args.json.write_text(json.dumps(result, indent=2) + "\n")
        if args.markdown:
            args.markdown.write_text(alerts_markdown(rows, result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
