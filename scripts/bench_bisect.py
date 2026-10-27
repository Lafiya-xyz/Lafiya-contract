#!/usr/bin/env python3
"""Find the first commit on a range where a benchmark metric regressed.

Re-runs the measurement (scripts/bench_measure.sh: build the commit's contract
wasm, measure it with lafiya-bench) across the first-parent commits of
GOOD..BAD and binary-searches for the first one whose metric is more than
THRESHOLD above GOOD's. A commit that fails to build or lacks the metric is
skipped, like `git bisect skip`: the search tries its neighbours instead.

Usage:
    cargo build -p lafiya-bench
    scripts/bench_bisect.py --good SHA --bad SHA \\
        --metric attester-registry/add_attester/cpu_instructions [--threshold 0.05]

Pass --measure-cmd to run something else in each checkout. It must print
lafiya-bench JSON on stdout. The tests use it to bisect a synthetic
regression without building contracts.
"""

import argparse
import sys
from pathlib import Path

from bench_history import DEFAULT_MEASURE, ROOT, git, measure, ratio


class Bisector:
    def __init__(self, repo: Path, metric: str, cmd: str) -> None:
        self.repo, self.metric, self.cmd = repo, metric, cmd
        self.cache: dict[str, float | None] = {}

    def value(self, commit: str) -> float | None:
        if commit not in self.cache:
            result = measure(self.repo, commit, self.cmd)
            raw = None if result is None else result.get("metrics", {}).get(self.metric)
            self.cache[commit] = None if raw is None else float(raw)
            print(f"  {commit[:12]}  {self.cache[commit]}", file=sys.stderr)
        return self.cache[commit]


def bisect(b: Bisector, good: str, bad: str, threshold: float) -> tuple[str | None, float]:
    """Return (first regressed commit or None, the good reference value)."""
    reference = b.value(good)
    if reference is None:
        raise SystemExit(f"cannot measure {b.metric} at good commit {good}")
    commits = git(b.repo, "rev-list", "--first-parent", "--reverse", f"{good}..{bad}").split()
    if not commits:
        raise SystemExit(f"{good}..{bad} is empty")

    def regressed(commit: str) -> bool | None:
        v = b.value(commit)
        return None if v is None else ratio(v, reference) > threshold

    # Invariant: every measurable commit before lo is good; commits[hi] is bad.
    lo, hi = 0, len(commits) - 1
    if not regressed(commits[hi]):
        return None, reference
    while lo < hi:
        mid = (lo + hi) // 2
        # Probe mid, or the nearest measurable commit to it (skip the rest).
        order = sorted(range(lo, hi), key=lambda i: (abs(i - mid), i))
        probes = ((i, v) for i in order if (v := regressed(commits[i])) is not None)
        probe = next(probes, None)
        if probe is None:
            # Only unmeasurable commits left before hi: the regression is hi
            # or one of them; main() lists the skipped commits.
            break
        if probe[1]:
            hi = probe[0]
        else:
            lo = probe[0] + 1
    return commits[hi], reference


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--good", required=True)
    parser.add_argument("--bad", default="HEAD")
    parser.add_argument("--metric", required=True)
    parser.add_argument("--threshold", type=float, default=0.05)
    parser.add_argument("--measure-cmd", default=DEFAULT_MEASURE)
    args = parser.parse_args(argv)

    b = Bisector(args.repo, args.metric, args.measure_cmd)
    first, reference = bisect(b, args.good, args.bad, args.threshold)
    if first is None:
        print(f"no regression above {args.threshold:.0%} in {args.good}..{args.bad}")
        return 1
    value = b.cache[first]
    subject = git(args.repo, "show", "-s", "--format=%h %s", first)
    print(f"first regressed commit: {subject}")
    print(f"{args.metric}: {reference:,.0f} -> {value:,.0f} (+{ratio(value or 0, reference):.1%})")
    skipped = sorted(c[:12] for c, v in b.cache.items() if v is None)
    if skipped:
        print(f"skipped (unmeasurable, could also be the culprit): {', '.join(skipped)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
