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

## `__check_auth` signer budget

`multisig-account` bounds an authorization by two constants:

| Constant | Value | Where |
|---|---|---|
| `MAX_SIGNERS` | 8 | `crates/multisig-account/src/lib.rs` |
| `MAX_CLIENT_DATA_LEN` | 512 | `crates/multisig-account/src/lib.rs` |

Both are derived from the worst-case `__check_auth` measurement below, not
picked by feel. The rule: a worst-case authorization must use at most **50%**
of the transaction CPU limit in `config/network-limits.json`, leaving the
other half for the rest of the transaction and for headroom against future
host-cost changes.

### Why the worst case is all passkeys

`secp256r1` verification is far more expensive than `ed25519`, and a passkey
signer also pays SHA-256 over the WebAuthn client data. The worst case is
therefore every signer a passkey with maximum-size client data, and every
role evaluated. Mixed-key matrices only ever come in under this bound.

### Matrix

Measured with `lafiya-bench` (`multisig-account/check_auth_*` scenarios),
plain `cargo build --release`, against the CPU limit in
`config/network-limits.json`. `cpu` is the fraction of the transaction CPU
limit; `mem` is the fraction of the memory limit. `ed25519` and `passkey`
columns are the signer mix; `client data` is the WebAuthn client-data size in
bytes (passkey signers only).

| Signers | Mix | Client data | cpu | mem |
|---:|---|---:|---:|---:|
| 1 | 1 ed25519 | 0 | 0.4% | 0.3% |
| 1 | 1 passkey | 512 | 6.1% | 1.2% |
| 2 | 1 ed25519 + 1 passkey | 512 | 6.5% | 1.6% |
| 4 | 2 ed25519 + 2 passkey | 512 | 12.8% | 2.9% |
| 4 | 4 passkey | 512 | 24.6% | 4.1% |
| 8 | 4 ed25519 + 4 passkey | 512 | 25.4% | 5.6% |
| 8 | 8 passkey | 512 | **49.1%** | 8.0% |
| 9 | 9 passkey | 512 | 55.2% | 9.0% |
| 8 | 8 passkey | 1024 | 49.6% | 8.4% |
| 8 | 8 passkey | 2048 | 50.4% | 9.2% |

### Derivation

- **`MAX_SIGNERS = 8`.** Eight all-passkey signers with maximum client data
  is the largest count that stays under the 50% CPU rule (49.1%). Nine
  signers crosses it (55.2%), so the bound is eight.
- **`MAX_CLIENT_DATA_LEN = 512`.** At eight passkey signers, 512 bytes of
  client data stays under the rule (49.1%). 1024 bytes is still under it
  (49.6%) but leaves almost no margin, and 2048 bytes crosses it (50.4%).
  512 is the largest round size with meaningful headroom, and it covers the
  WebAuthn client data a real authenticator produces.

The numbers above are the reference point. The worst-case row (8 passkey
signers, 512-byte client data) is checked in CI by
`multisig-account/tests/budget.rs`, which fails if that authorization uses
more than 50% of the transaction CPU limit. If a host-cost change moves the
numbers, re-measure, update this table, and adjust the constants and the test
together.
