#!/usr/bin/env python3
"""Render the wasm size and resource-budget delta report posted on PRs.

    python3 scripts/ci/size_report.py \
        --base-wasm base/ --head-wasm head/ \
        --base-budgets base-baseline.json --head-budgets budgets/baseline.json \
        [--twiggy-dir twiggy/] [--threshold 5] > report.md

Compares each contract's release `.wasm` size and every per-entry-point
budget metric (`budgets/baseline.json`) between the base branch and the PR
head. Rows that grow by more than `--threshold` percent are marked as
regressions; `--fail-on-regression` turns them into a non-zero exit.
`--twiggy-dir` embeds the `twiggy diff` / `twiggy top` output (one
`<contract>.diff.txt` / `<contract>.top.txt` per contract) to attribute the
growth to functions and data segments.
"""

import argparse
import json
import sys
from pathlib import Path

MARKER = "<!-- lafiya-size-report -->"


def wasm_sizes(directory: Path) -> dict[str, int]:
    return {p.stem: p.stat().st_size for p in sorted(directory.glob("*.wasm"))}


def budget_metrics(path: Path) -> dict[tuple[str, str, str], int]:
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    return {
        (contract, entry, metric): value
        for contract, entries in data.items()
        for entry, metrics in entries.items()
        for metric, value in metrics.items()
        if isinstance(value, int)
    }


def delta(base: int | None, head: int | None, threshold: float) -> tuple[str, str, bool]:
    """Return (change, percent, is_regression) cells for one row."""
    if base is None:
        return "new", "", False
    if head is None:
        return "removed", "", False
    diff = head - base
    pct = (diff / base * 100) if base else 0.0
    regression = pct > threshold
    return f"{diff:+,}", f"{pct:+.1f}%", regression


def mark(regression: bool, diff: str) -> str:
    if regression:
        return ":red_circle: regression"
    if diff.startswith("-"):
        return ":green_circle:"
    return ""


def render(args: argparse.Namespace) -> tuple[str, int]:
    base_sizes, head_sizes = wasm_sizes(args.base_wasm), wasm_sizes(args.head_wasm)
    base_budgets = budget_metrics(args.base_budgets)
    head_budgets = budget_metrics(args.head_budgets)
    regressions = 0

    lines = [MARKER, "## Wasm size and resource budgets", ""]
    lines.append(f"Rows growing by more than **{args.threshold:g}%** vs. the base branch are flagged.")
    lines += ["", "### Wasm size", "", "| Contract | Base | Head | Change | % | |", "|---|--:|--:|--:|--:|---|"]
    for name in sorted(base_sizes.keys() | head_sizes.keys()):
        base, head = base_sizes.get(name), head_sizes.get(name)
        diff, pct, regression = delta(base, head, args.threshold)
        regressions += regression
        lines.append(
            f"| `{name}` | {base or 0:,} | {head or 0:,} | {diff} | {pct} | {mark(regression, diff)} |"
        )

    changed = [
        key
        for key in sorted(base_budgets.keys() | head_budgets.keys())
        if base_budgets.get(key) != head_budgets.get(key)
    ]
    lines += ["", "### Resource budgets (per entry point)", ""]
    if not changed:
        lines.append("No budget metric changed.")
    else:
        lines += ["| Contract | Entry point | Metric | Base | Head | Change | % | |", "|---|---|---|--:|--:|--:|--:|---|"]
        for contract, entry, metric in changed:
            base = base_budgets.get((contract, entry, metric))
            head = head_budgets.get((contract, entry, metric))
            diff, pct, regression = delta(base, head, args.threshold)
            regressions += regression
            lines.append(
                f"| `{contract}` | `{entry}` | {metric} | {base or 0:,} | {head or 0:,} "
                f"| {diff} | {pct} | {mark(regression, diff)} |"
            )

    if args.twiggy_dir and args.twiggy_dir.is_dir():
        lines += ["", "### Size attribution (twiggy)", ""]
        for report in sorted(args.twiggy_dir.glob("*.txt")):
            text = report.read_text(encoding="utf-8").strip()
            if text:
                lines += [f"<details><summary><code>{report.name}</code></summary>", "", "```", text, "```", "", "</details>"]

    lines += ["", f"**{regressions} regression(s)** above threshold." if regressions else "No regressions above threshold."]
    return "\n".join(lines) + "\n", regressions


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base-wasm", type=Path, required=True)
    parser.add_argument("--head-wasm", type=Path, required=True)
    parser.add_argument("--base-budgets", type=Path, required=True)
    parser.add_argument("--head-budgets", type=Path, required=True)
    parser.add_argument("--twiggy-dir", type=Path)
    parser.add_argument("--threshold", type=float, default=5.0, help="regression threshold in percent")
    parser.add_argument("--fail-on-regression", action="store_true")
    args = parser.parse_args()
    report, regressions = render(args)
    sys.stdout.write(report)
    if regressions and args.fail_on_regression:
        sys.exit(1)


if __name__ == "__main__":
    main()
