# Comprehensive Testing Architecture for Lafiya Contracts

## Four-Layer Testing Strategy

### Layer 1: Budget Regression Gates (#374)
**Purpose:** Catch performance regressions before they accumulate
- Baseline: `budgets/baseline.json` with CPU, memory, I/O for each entry point
- CI check: >5% regression fails the build
- Owner: DevOps/CI

**Files:**
- `budgets/baseline.json` — baseline metrics
- `tests/budget_regression_test.rs` — regression test harness

### Layer 2: Formal Verification (#371)
**Purpose:** Prove invariants hold for all inputs
- Tool: Kani (Rust model checker)
- Invariants: attester existence, count integrity, admin control, threshold auth
- CI: Nightly or on critical path changes
- Owner: Security/QA

**Files:**
- `formal-verification/invariants.md` — specs and proofs
- `contracts/formal-verification/src/lib.rs` — verifiable functions

### Layer 3: Mutation Testing (#372)
**Purpose:** Measure test quality; ensure tests catch bugs
- Tool: cargo-mutants
- Target: Zero survivors for auth/pause checks
- CI: On PR, fails if new survivors found
- Owner: QA/Engineering

**Files:**
- `.cargo/mutants.toml` — configuration and exclusions
- `MUTATION_TRIAGE.md` — triage log

### Layer 4: Differential Testing (#373)
**Purpose:** Verify release WASM behaves identically to native
- Method: Run same proptest sequences against both
- Check: Return values, errors, events, storage, budgets match
- Owner: Build/Release

**Files:**
- `tests/differential_test.rs` — harness

## Implementation Timeline

| Week | Task | Owner |
|------|------|-------|
| 1 | Budget baseline collection | DevOps |
| 2 | Formal verification specs | Security |
| 3 | Mutation testing triage | QA |
| 4 | Differential testing setup | Build |

## Expected Results

- **Budget:** Catch 20% regressions before merge
- **Verification:** Prove 7 core invariants hold
- **Mutation:** 95%+ kill rate on critical functions
- **Differential:** 100% native/WASM equivalence

## CI/CD Integration

```yaml
.github/workflows/testing-full.yml
├── budget-regression
├── formal-verification (nightly)
├── mutation-testing (PR, critical-files)
└── differential-testing (release builds)
```

## Incentive pool property suites (#507, #508)

- `contracts/incentive-pool/src/stateful_test.rs` — model-based stateful
  harness (fund/withdraw/approve/claim/pause/caps/suspension). Asserts custody
  `balance == deposited - paid`, `total_paid <= total_deposited`, at-most-once
  payout and per-attester cap. Failing traces print as a `Vec<Op>` that can be
  passed to `replay(...)`.
- `contracts/incentive-pool/src/fuzz_test.rs` — one target per mutating
  function plus a sequence target; picked up by the CI `fuzz` job
  (`test(fuzz_test)`, 256 cases).
- CI time impact: default 64 cases per property adds a few seconds to the
  `test` job; the `fuzz` job runs at 256 cases. The extended profile
  (`PROPTEST_CASES=10000`) runs nightly in
  `.github/workflows/incentive-pool-nightly.yml` (45 min budget) and uploads
  `proptest-regressions/` on failure.
- Reproduce locally:
  `PROPTEST_CASES=10000 cargo test -p incentive-pool -- stateful_test fuzz_test --nocapture`.

## Proptest seeding, tiers, and regression persistence (#721)

### Case-count tiers

Proptest case counts are standardized through `PROPTEST_CASES` and the nextest
profiles in `.config/`:

| Tier | Profile | `PROPTEST_CASES` | When it runs |
|------|---------|------------------|--------------|
| PR (fast) | `default` | 256 | Every pull request |
| Nightly (deep) | `nightly` | 50000 | Scheduled nightly workflow |

Run the PR tier locally with `cargo nextest run` (the default profile). Run the
nightly tier with `cargo nextest run --profile nightly`. Both profiles set
`PROPTEST_CASES`; do not hard-code case counts in individual tests.

### Seeds and failure output

Every proptest failure prints its seed and the minimal failing input. Capture
the seed from the failure output — it is also included in the deduplicated
nightly issue opened automatically when a nightly run fails.

### Regression files

Every crate with proptests commits its `proptest-regressions/` directory
(`contracts/multisig-account/proptest-regressions/` is the existing example).
CI fails when a regression file has uncommitted changes, so a failure found in
CI or nightly is never lost: commit the generated file in the fixing PR.

### Reproducing a failure locally

1. Re-run with the failing seed:
   `PROPTEST_CASES=256 PROPTEST_RNG_SEED=<seed> cargo test -p <crate> -- <test_name> --nocapture`
2. Or replay the persisted regression file:
   `cargo test -p <crate> -- <test_name> --nocapture`
   (proptest automatically replays cases from `proptest-regressions/`).
3. To reproduce the deep tier, use `PROPTEST_CASES=50000` (or the `nightly`
   nextest profile) with the same seed.

## SystemBuilder migration for contract tests (#722)

`lafiya-test-utils` provides a `SystemBuilder` that deploys any subset of the
five contracts wired as in production (via constructors), exposing typed
handles and named actors.

- Prefer `SystemBuilder` over ad-hoc `setup()` helpers and raw `register`
  calls in `contracts/*/src/test.rs`, `fuzz_test.rs`, `large_test.rs`, and
  `tests/*.rs`.
- Migrate one contract per commit with no change in assertions; the test count
  must stay the same.
- New tests must use `SystemBuilder`; a lint/check discourages new ad-hoc
  `register`/setup code in tests.
- Report test LOC before/after in the migration PR (target: ≥80% of setup code
  migrated, measurable LOC decrease).
