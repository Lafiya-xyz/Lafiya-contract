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
