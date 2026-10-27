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
