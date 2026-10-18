# CI

`.github/workflows/ci.yml` runs on every push to `main`, every pull request, and nightly.

## Required check

Branch protection should require **only** the `ci-success` check. It is an aggregator
job that depends on every other CI job and passes when each of them either succeeded or
was skipped by the path filter. Any failure or cancellation fails it. New jobs must be added
to its `needs` list.

## Job graph

| Job | Runs when | Purpose |
|---|---|---|
| `changes` | always | `dorny/paths-filter`: sets `rust=true` when contract, crate, test, Cargo, toolchain, nextest, or CI files change. On PRs it also selects the affected crates (`test-scope`). |
| `shell-lint`, `docs-checks` | always | Shell lint; README tables, verification-model truth table. |
| `golden` | always | `cargo xtask generate --check`: golden snapshots of every generated artifact (see below). |
| `fmt`, `clippy`, `build-wasm` | `rust` changed | As before. `build-wasm` uploads the release wasm (`release-wasm` artifact) for downstream jobs to reuse. |
| `build-tests` | `rust` changed | Compiles the test binaries in `test-scope` once (`cargo nextest archive --all-features`). |
| `test` (2 partitions) | after `build-tests` | Runs the archive with `--partition count:N/2`, profile `ci`, and uploads JUnit XML. |
| `fuzz` | after `build-tests` | Proptest cases from the same archive (non-blocking). |
| `test-report` | after `test` | Shows JUnit results on the PR (`dorny/test-reporter`). |
| `timings` | always, last | Per-job durations as a summary table and a `job-timings` JSON artifact. |

Docs-only PRs skip the Rust jobs; `ci-success` still reports.

The previous workflow ran `cargo test --workspace` twice (default and `--all-features`).
The workspace's only feature is `testutils`, which test builds already enable, so the
single `--all-features` archive covers both.

## nextest

Configuration lives in `.config/nextest.toml`:

- `ci` profile: no fail-fast, per-test timeout (terminate after 5 minutes; 10 for
  `fuzz_test`), JUnit output at `target/nextest/ci/junit.xml`.
- **No blanket retries.** A known-flaky test can be made retry-tolerant with an explicit
  `[[profile.ci.overrides]]` entry that links its tracking issue.

Locally: `cargo install cargo-nextest`, then `cargo nextest run --workspace --all-features`.

## Flaky-test detection

`.github/workflows/flaky-tests.yml` runs nightly (and on demand). It runs the suite N times
(default 5) with `--retries 0` and opens or comments on a "Flaky tests detected" issue
listing each test that both passed and failed.

## Caching

The compiling jobs (`clippy`, `build-tests`, `build-wasm`, `golden`) run rustc through
`sccache` with the GitHub Actions cache backend (`SCCACHE_GHA_ENABLED`), so compiled
crates are shared across jobs and runs instead of each job restoring its own `target/`.
`Swatinem/rust-cache` is kept with `cache-targets: false` for the registry and git
checkouts only. `build-tests` writes `sccache --show-stats` to its job summary.

## Affected-crate test selection

On pull requests `scripts/ci/affected_crates.py` maps the diff to the workspace crates that
own the changed files and adds their workspace dependents, and `build-tests` archives only
those (`-p ...`). Changes to the workspace manifest, lockfile, toolchain, `.cargo/`,
`.config/`, root `tests/` or `ci.yml` select `--workspace`. Pushes to `main`, the nightly
schedule and manual runs always test the full workspace.

```bash testable
cd "$REPO_ROOT"
# An empty diff selects the full workspace.
python3 scripts/ci/affected_crates.py HEAD | grep -qx -- "--workspace"
```

## Golden snapshots

`cargo xtask generate` regenerates every committed generated artifact: interface snapshots
(`scripts/conformance/snapshots/`), TypeScript bindings (`bindings/*/src/index.ts`), the
error/event catalogs, `docs/events.md`, the `@lafiya/events` decoders, the indexer OpenAPI
operation index (`golden/openapi-operations.txt`) and the release manifest
(`golden/release-manifest.json`, with commit, timestamp and wasm hash/size masked).
`--check` restores the committed files and prints a semantic diff for anything that
drifted: one `~`/`-`/`+` line per changed JSON key path, changed lines for other files.
After a deliberate change, run `cargo xtask generate` and commit the result.

## Size and budget report

`.github/workflows/size-report.yml` builds the contracts at the PR head and the base commit
and posts a sticky PR comment (`scripts/ci/size_report.py`) with each contract's wasm size
delta and every per-entry-point metric in `budgets/baseline.json` that changed. Rows
growing more than 5% are marked as regressions, and `twiggy diff`/`twiggy top` output
attributes the growth. Budgets are still enforced pass/fail elsewhere; the report is for
review.

## Docs quality gates

The `quality` job in `.github/workflows/docs.yml` runs on docs PRs:

- **Links**: lychee over all Markdown (`lychee.toml`), with results cached in
  `.lycheecache` between runs. Known-good URLs that CI cannot reach go in `.lycheeignore`.
- **Spelling**: `typos` with the project dictionary in `_typos.toml`
  (`[default.extend-words]`).
- **Snippets**: `scripts/docs/run_snippets.py` runs every fenced `bash`/`sh` or `ts` block
  whose info string adds `testable` (```` ```bash testable ````) in a fresh temporary
  directory with a minimal environment; `$REPO_ROOT` points at the checkout. Use
  `--list` to see which snippets are picked up.

## Wall-clock time

Record before/after numbers from the Actions "Usage" view for a typical contract PR. The
target is a 30%+ reduction.

| | Wall clock | Notes |
|---|---|---|
| Before (single `test` job, 2 full test runs) | _to be recorded_ | |
| After (archive + 2 partitions) | _to be recorded_ | |
