# CI

`.github/workflows/ci.yml` runs on every push to `main` and every pull request.

## Required check

Branch protection should require **only** the `ci-success` check. It is an aggregator
job that depends on every other CI job and passes when each of them either succeeded or
was skipped by the path filter. Any failure or cancellation fails it. New jobs must be added
to its `needs` list.

## Job graph

| Job | Runs when | Purpose |
|---|---|---|
| `changes` | always | `dorny/paths-filter`: sets `rust=true` when contract, crate, test, Cargo, toolchain, nextest, or CI files change. |
| `shell-lint`, `docs-checks` | always | Shell lint; README tables, error/event catalog drift, verification-model truth table. |
| `fmt`, `clippy`, `build-wasm` | `rust` changed | As before. `build-wasm` uploads the release wasm (`release-wasm` artifact) for downstream jobs to reuse. |
| `build-tests` | `rust` changed | Compiles all test binaries once (`cargo nextest archive --all-features`). |
| `test` (2 partitions) | after `build-tests` | Runs the archive with `--partition count:N/2`, profile `ci`, and uploads JUnit XML. |
| `fuzz` | after `build-tests` | Proptest cases from the same archive (non-blocking). |
| `test-report` | after `test` | Shows JUnit results on the PR (`dorny/test-reporter`). |

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

The workflow keeps `Swatinem/rust-cache`. `sccache` with the GitHub Actions cache backend
was considered: it helps most when many jobs compile the same crates, but after the
build-once change only `clippy`, `build-tests`, and `build-wasm` compile, each with
different flags, so the expected gain is small against the extra cache-API traffic. Revisit
if compile jobs multiply; record the measured numbers here.

## Wall-clock time

Record before/after numbers from the Actions "Usage" view for a typical contract PR. The
target is a 30%+ reduction.

| | Wall clock | Notes |
|---|---|---|
| Before (single `test` job, 2 full test runs) | _to be recorded_ | |
| After (archive + 2 partitions) | _to be recorded_ | |
