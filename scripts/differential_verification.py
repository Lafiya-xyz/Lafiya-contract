#!/usr/bin/env python3
"""Differential testing of the verification model across implementations.

    python3 scripts/differential_verification.py                  # fixed seeds (CI on every PR)
    python3 scripts/differential_verification.py --random         # random seed (nightly sweep)
    python3 scripts/differential_verification.py --seeds 7 --samples 2000

Generates scenarios from docs/specs/verification-model.json (every truth-table
case, the full input space while it is small enough, and seeded random
samples), runs the Python reference (scripts/verification_model.py), the Rust
crate (crates/lafiya-verification-model) and the TypeScript SDK
(packages/verifier-sdk/src/model.ts) on each one, and diffs verdict, reason,
and qualifier. On any discrepancy it shrinks the scenario to a minimal set of
overrides from `default_input` and exits 1. It also reports which model
verdicts and reasons were exercised and fails if a verdict was never reached.

Implementations other than the reference are external commands speaking
JSON lines on stdin/stdout; override them with `--impl name=command`.
"""

from __future__ import annotations

import argparse
import itertools
import json
import os
import random
import shlex
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from verification_model import TABLE, explain

ROOT = Path(__file__).resolve().parents[1]

Scenario = dict[str, Any]
Outcome = dict[str, Any]

DEFAULT_IMPLS = {
    "rust": "cargo run --quiet --locked -p lafiya-verification-model",
    "ts": "node packages/verifier-sdk/scripts/model-eval.mjs",
}
DEFAULT_SEEDS = "1,2,3,4,5"


def input_domains(table: dict[str, Any]) -> dict[str, list[Any]]:
    """Values each input can take, derived from the model itself.

    Booleans take both values; enum (string) inputs take every value that
    appears in `default_input` or any case, so a new enum value is generated
    as soon as one truth-table case uses it.
    """
    domains: dict[str, list[Any]] = {}
    for key, default in table["default_input"].items():
        if isinstance(default, bool):
            domains[key] = [False, True]
            continue
        seen = {default} | {c["input"][key] for c in table["cases"] if key in c["input"]}
        domains[key] = sorted(seen)
    return domains


def generate(
    table: dict[str, Any], seeds: list[int], samples: int, max_exhaustive: int
) -> list[Scenario]:
    defaults = table["default_input"]
    domains = input_domains(table)
    scenarios: list[Scenario] = [{**defaults, **c["input"]} for c in table["cases"]]
    keys = list(domains)
    space = 1
    for values in domains.values():
        space *= len(values)
    if space <= max_exhaustive:
        for combo in itertools.product(*(domains[k] for k in keys)):
            scenarios.append(dict(zip(keys, combo, strict=True)))
    for seed in seeds:
        rng = random.Random(seed)
        for _ in range(samples):
            scenarios.append({k: rng.choice(domains[k]) for k in keys})
    return scenarios


def reference(scenario: Scenario) -> Outcome:
    verdict, reason, qualifier = explain(scenario)
    return {"verdict": verdict, "reason": reason, "qualifier": qualifier}


def run_external(command: str, scenarios: list[Scenario]) -> list[Outcome]:
    stdin = "".join(json.dumps(s) + "\n" for s in scenarios)
    proc = subprocess.run(
        shlex.split(command),
        input=stdin,
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"`{command}` exited {proc.returncode}:\n{proc.stderr}")
    lines = [line for line in proc.stdout.splitlines() if line.strip()]
    if len(lines) != len(scenarios):
        raise RuntimeError(f"`{command}` returned {len(lines)} results for {len(scenarios)} inputs")
    return [json.loads(line) for line in lines]


def diff(scenario: Scenario, impls: dict[str, str]) -> dict[str, Outcome] | None:
    """Outcomes per implementation if they disagree on `scenario`, else None."""
    outcomes = {"reference": reference(scenario)}
    for name, command in impls.items():
        outcomes[name] = run_external(command, [scenario])[0]
    return outcomes if len({json.dumps(o, sort_keys=True) for o in outcomes.values()}) > 1 else None


def minimize(scenario: Scenario, defaults: Scenario, impls: dict[str, str]) -> Scenario:
    """Greedily reset inputs to their defaults while the discrepancy persists."""
    current = dict(scenario)
    changed = True
    while changed:
        changed = False
        for key, default in defaults.items():
            if current[key] == default:
                continue
            candidate = {**current, key: default}
            if diff(candidate, impls) is not None:
                current = candidate
                changed = True
    return current


def overrides(scenario: Scenario, defaults: Scenario) -> Scenario:
    return {k: v for k, v in scenario.items() if defaults[k] != v}


def coverage_report(table: dict[str, Any], outcomes: list[Outcome]) -> tuple[str, list[str]]:
    verdicts = Counter(o["verdict"] for o in outcomes)
    reasons = Counter(o["reason"] for o in outcomes)
    missing = [v for v in table["verdicts"] if verdicts[v] == 0]
    lines = [
        f"### Verification-model differential coverage (model {table['model_version']})",
        "",
        "| Verdict | Scenarios |",
        "|---|---|",
        *(f"| `{v}` | {verdicts[v]} |" for v in table["verdicts"]),
        "",
        "| Reason | Scenarios |",
        "|---|---|",
        *(f"| `{r}` | {n} |" for r, n in sorted(reasons.items())),
        "",
    ]
    return "\n".join(lines), missing


def parse_impls(values: list[str] | None) -> dict[str, str]:
    if not values:
        return dict(DEFAULT_IMPLS)
    impls = {}
    for value in values:
        name, _, command = value.partition("=")
        if not command:
            raise SystemExit(f"--impl expects name=command, got {value!r}")
        impls[name] = command
    return impls


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    parser.add_argument("--seeds", default=DEFAULT_SEEDS, help="comma-separated RNG seeds")
    parser.add_argument("--random", action="store_true", help="use one fresh random seed")
    parser.add_argument("--samples", type=int, default=500, help="random scenarios per seed")
    parser.add_argument(
        "--max-exhaustive",
        type=int,
        default=1 << 16,
        help="enumerate the whole input space while it has at most this many points",
    )
    parser.add_argument("--impl", action="append", help="name=command (repeatable)")
    args = parser.parse_args(argv)

    table = json.loads(TABLE.read_text())
    defaults: Scenario = table["default_input"]
    if args.random:
        seeds = [int.from_bytes(os.urandom(4), "big")]
    else:
        seeds = [int(s) for s in args.seeds.split(",") if s.strip()]
    impls = parse_impls(args.impl)
    print(f"[differential] seeds={','.join(map(str, seeds))} impls=reference,{','.join(impls)}")

    scenarios = generate(table, seeds, args.samples, args.max_exhaustive)
    expected = [reference(s) for s in scenarios]
    results = {name: run_external(cmd, scenarios) for name, cmd in impls.items()}

    failures = 0
    for index, scenario in enumerate(scenarios):
        bad = [name for name, outs in results.items() if outs[index] != expected[index]]
        bad += [n for n, outs in results.items() if outs[index]["verdict"] not in table["verdicts"]]
        if not bad:
            continue
        failures += 1
        minimal = minimize(scenario, defaults, impls)
        outcomes = diff(minimal, impls) or {}
        print(f"[differential] DISCREPANCY ({', '.join(sorted(set(bad)))}); minimal scenario:")
        print("  input: " + json.dumps(overrides(minimal, defaults), sort_keys=True))
        for name, outcome in outcomes.items():
            print(f"  {name:>9}: {json.dumps(outcome, sort_keys=True)}")
        break  # one minimal repro is enough to act on

    report, missing = coverage_report(table, expected)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as fh:
            fh.write(f"Seeds: `{','.join(map(str, seeds))}`\n\n{report}\n")
    if missing:
        print(f"[differential] verdicts never reached: {', '.join(missing)}")
    status = "FAIL" if failures or missing else "ok"
    print(f"[differential] {len(scenarios)} scenarios, {status}")
    return 1 if failures or missing else 0


if __name__ == "__main__":
    sys.exit(main())
