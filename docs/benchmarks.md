# Benchmark history and regression bisect

`budgets/baseline.json` holds only the latest allowed state. Each PR is
compared with it, so a footprint that grows a little with every PR never
trips a per-PR threshold. The benchmark history records every commit on
`main` so that kind of drift shows up.

## Pieces

| Piece | What it does |
|---|---|
| `crates/lafiya-bench` | Harness. It loads each contract from its built `.wasm` (not from the crate), runs a fixed set of scenarios, and prints a flat JSON object. Keys look like `contract/scenario/metric`, for example `attester-registry/add_attester_loaded/cpu_instructions`. Metrics: CPU instructions, memory, ledger read/write entries and bytes, event size, and wasm size. |
| `scripts/bench_measure.sh` | Builds the contract wasm of the checkout in the current directory and runs the harness on it. |
| `scripts/bench_history.py` | `record`, `backfill`, `alerts` and `render` over the history. |
| `scripts/bench_bisect.py` | Re-runs the measurement across a commit range to find the commit that introduced a regression. |
| `budgets/trend.json` | Alert tolerances, the metrics tracked, and acknowledged increases. |
| `.github/workflows/bench-history.yml` | Runs on every push to `main`. |

The harness reads wasm, not crates, so the copy built from today's tree can
measure any older commit. A scenario whose entry point did not exist yet, or
had a different signature, is listed under `errors` and leaves a gap in that
series. It does not fail the run.

Every commit is built the same way: plain `cargo build --release`, not the
reproducible release build. Absolute numbers can differ from release
artifacts, but the series stays comparable.

## Where the data lives

Each commit gets one file, `commits/<sha>.json`, on the `bench-history` data
branch. The file holds the commit, its first-parent position on `main`, the
date, the subject, the metrics and any scenario errors. `index.html` on the
same branch is the rendered trend page. Each workflow run also uploads it as
the `bench-trend` artifact.

On its first run the workflow backfills the last 50 first-parent commits of
`main`. After that it measures only commits that are missing, normally just
the one that was pushed. To backfill further, run the workflow by hand and
set `backfill` to the number of commits. Commits that do not build are
skipped.

## Trend page

The page shows one small chart per scenario for the selected metric, a
tooltip per commit, and a table view. A scenario with an active drift alert
gets a ⚠ badge.

## Drift alerts and hysteresis

Contract metrics are deterministic, but a single PR can still bump a number
and the next one can bring it back. To keep alerts from flapping:

- **Raise:** a metric raises once it is more than `tolerance` (5%) above the
  median of its previous `lookback` (20) points for `sustain` (3)
  consecutive commits. One spike never raises. A slow drift, like +1% per
  commit, raises once it adds up.
- **Hold:** while raised, the alert keeps the reference value it raised
  against. It does not fade as the moving median catches up.
- **Clear:** the alert clears only after `sustain` consecutive commits are
  back within `clear_tolerance` (2%) of that reference. A value between 2%
  and 5% keeps the alert raised.
- **Acknowledge:** if the increase is intended, add
  `"<metric key>": "<sha>"` to `acknowledged` in `budgets/trend.json` in a
  reviewed PR. That metric's series then restarts at that commit.

The workflow keeps a single `bench-drift` issue in sync with the alerts. It
opens the issue when an alert raises, updates its body while alerts are
active, and closes it when they clear. Because of the hysteresis, the issue
changes state only when the drift itself does.

## Bisecting a regression

```bash
cargo build -p lafiya-bench
scripts/bench_bisect.py --good <sha before drift> --bad main \
  --metric attester-registry/add_attester_loaded/cpu_instructions --threshold 0.05
```

The script walks the first-parent commits of `GOOD..BAD`. Each probe gets
its own temporary worktree. The script binary-searches for the first commit
whose metric is more than `--threshold` above GOOD's value. Commits that fail
to build, or lack the metric, are skipped the way `git bisect skip` skips
them: the nearest measurable commit is probed instead, and any skipped
commit that could be the culprit is listed. Results are cached per commit.

`--measure-cmd` swaps out the build-and-measure step. The tests use it to
bisect a synthetic regression planted at a known commit, with and without
unbuildable commits around it (`scripts/tests/test_bench.py`).

To measure the current checkout without the history:

```bash
cargo build -p lafiya-bench
scripts/bench_measure.sh | jq .metrics
```

## Populated-state benchmarks

Benchmarks against empty or tiny state underestimate production costs: read
and write fees depend on entry sizes, and some operations depend on collection
sizes (history, co-attesters, affiliation periods, rate windows). The harness
therefore also runs a set of populated-state scenarios. They seed state to
realistic target scales through direct storage writes in `testutils` (synthetic
data only, never real health data or QR payloads), then measure the hot paths.

| Scale | Target |
|---|---|
| Attesters | 10k |
| Attestations | 1M (simulated through direct storage writes) |
| Facilities | 500 |
| Histories | full (no truncation) |

These scenarios run in the `bench-history` workflow alongside the base
scenarios, so their series are recorded per commit like any other metric. Their
keys carry a `populated/` prefix, for example
`attester-registry/populated/add_attester_10k/cpu_instructions`.

## Growth assertions

For operations that touch collections, the harness measures the cost at
several state sizes and asserts the growth class the design claims. A scenario
declares its expected class:

- **O(1):** cost must stay flat as the collection grows (for example, a
  lookup by key).
- **O(log n):** cost may grow logarithmically (for example, a bounded search
  over a sorted structure).

The assertion fits the measured points and fails the run when the observed
growth exceeds the declared class beyond a tolerance. This catches accidental
superlinear behavior (for example, a scan that turns a lookup into O(n)) that
a single-size benchmark would miss. The collection-touching operations covered
are history reads, co-attester resolution, affiliation-period lookups and
rate-window updates.

## Feeding the capacity model

The measured values feed `budgets/capacity-model.json`, which
`docs/capacity-forecast.md` renders. The populated-state numbers replace the
synthetic minimal inputs, so the forecast reflects production-scale entry
sizes and collection sizes. When a populated-state scenario changes its
measured cost, the capacity model is regenerated from the latest history so
the forecast tracks the measured values rather than hand-entered estimates.
