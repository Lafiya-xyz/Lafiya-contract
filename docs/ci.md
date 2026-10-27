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
| `indexer` | `rust` changed | Builds `lafiya-indexer`, runs `clippy -D warnings`, and runs its tests (including the e2e suite) against a Postgres service container. |
| `qr` | `rust` changed | Builds `lafiya-qr`, runs `clippy -D warnings`, and runs its tests. |
| `deny` | `rust` changed | `cargo deny check` across the whole workspace, covering `lafiya-indexer` and `lafiya-qr`. |

Docs-only PRs skip the Rust jobs; `ci-success` still reports.

## Workspace crates

`crates/lafiya-indexer` and `crates/lafiya-qr` are first-class workspace members. They are
built, linted, tested, and audited on every PR like the other crates:

- **Build:** `cargo build -p lafiya-indexer` / `cargo build -p lafiya-qr`.
- **Clippy:** `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`.
- **Test:** `cargo nextest run -p <crate> --all-features`. The indexer e2e tests require a
  Postgres service; the `indexer` job provisions one and sets `DATABASE_URL` for the run.
- **Deny:** `cargo deny check` runs over the full workspace, so both crates are covered by
  the license, advisory, ban, and source checks.

Dependency versions for both crates are declared in `[workspace.dependencies]` at the repo
root. When adding a dependency, prefer the workspace entry over a crate-local version so
the two crates stay in lockstep with the rest of the workspace. `cargo tree -d` must not
show new duplicate major versions; any remaining duplicate must be justified in the PR.

## Required branch-protection checks

Branch protection should require **only** the `ci-success` aggregator. Because `ci-success`
depends on `indexer`, `qr`, and `deny`, those gates are enforced transitively. Do not add
the individual jobs to the required list; keep the single aggregator so path-filtered
skips keep working.
=======

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
