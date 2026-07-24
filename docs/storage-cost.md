# Storage Cost Benchmarks

This document tracks the resource-regression guard for the attester allowlist as it scales.

## Enforced checkpoints

`large_attester_allowlist_load` records the budget of `add_attester` after the
allowlist reaches each checkpoint and fails if either ceiling is exceeded.

| Operation | Attesters | Maximum CPU instructions | Maximum memory bytes |
|---|---:|---:|---:|
| `add_attester` | 10 | 2,000,000 | 1,000,000 |
| `add_attester` | 100 | 2,000,000 | 1,000,000 |
| `add_attester` | 1,000 | 2,000,000 | 1,000,000 |

## Methodology

The test initializes `attester-registry`, adds 1,000 distinct addresses, and
reads `Budget::cpu_instruction_cost` and `Budget::memory_bytes_cost` immediately
after additions 10, 100, and 1,000. Soroban resets metering before each
top-level invocation, so each checkpoint measures one `add_attester` call with
the allowlist at that size. This catches a size-dependent scan without conflating
the result with the cost of populating the preceding entries.

These are native-test regression measurements, not fee estimates. Native
registration omits Wasm execution, transaction-envelope processing, and some
ledger work, so the values must not be used to predict production fees.

Run the test and print its measured values with:

```sh
cargo test -p attester-registry large_attester_allowlist_load -- --nocapture
```

Last source verification: contract commit `82f7473`, with `soroban-sdk 27.0.0`
from the workspace lockfile. Update this reference and review the ceilings
whenever contract behavior or the pinned SDK cost model changes.
