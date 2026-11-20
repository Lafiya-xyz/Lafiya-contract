# Custom Dylint lints

Crate: `lints/` (own pinned nightly, not a workspace member). Run with
`cargo install cargo-dylint dylint-link` then, from the repo root:
`cargo dylint --path lints --all -- --workspace --all-targets`.
Suppress only with a reason, e.g.
`#[allow(storage_in_loop, reason = "bounded to 3 iterations")]`;
`scripts/check_lint_allows.sh` fails CI when the reason is missing.

| Lint | Rationale | Bad | Good |
|---|---|---|---|
| `persistent_set_without_ttl` | Persistent entries without `extend_ttl` expire/archive and break reads. | `s.persistent().set(k, v)` alone | `set` then `extend_ttl` in the same fn |
| `storage_in_loop` | Each storage access is metered; loops make cost unbounded. | `for k in keys { s.persistent().get(k) }` | read once outside the loop, or bound and annotate |
| `unchecked_amount_arithmetic` | `i128`/`u128` amounts must not silently wrap or panic. | `a + b` | `a.checked_add(b)?` |
| `unlimited_budget` | Tests must run under network-realistic budgets; `reset_unlimited` hides cost regressions that would fail on-chain. | `env.cost_estimate().budget().reset_unlimited()` | use `lafiya_test_utils::budgeted_env()` (see below) |

CI runtime: first run builds the lint crate and installs cargo-dylint (locally: about 1 min to build and test the lint crate, plus the lint pass over the workspace; cold CI time not yet measured); the cache in the `dylint` job removes most of it.

## `unlimited_budget` lint and the `reset_unlimited` allowlist

The `unlimited_budget` lint flags any call to `reset_unlimited` (and the
equivalent `Budget::reset_unlimited` path) outside an explicit allowlist.
Unlimited budgets are only legitimate in benchmarks, where the goal is to
measure raw instruction counts rather than enforce limits.

The allowlist lives in `lints/unlimited_budget_allowlist.txt`, one
repo-relative path per line (comments with `#`). Current entries:

```
# Benchmarks measure raw cost; they intentionally bypass limits.
benches/
```

`scripts/check_reset_unlimited.sh` is the grep-based backstop that runs in CI
alongside the Dylint job. It fails the build if `reset_unlimited` appears in
any tracked file not covered by the allowlist, and it also fails if the
allowlist itself references a path that no longer exists. To add a file, edit
the allowlist in the same PR and justify it in the description.

## Budget-constrained `Env` helper

`lafiya-test-utils` exposes `budgeted_env()`, which builds an `Env` whose CPU
and memory budgets are taken from `config/network-limits.json` (the same file
used to configure the network). Contract tests should construct their
environment through this helper instead of `Env::default()` so that any code
path exceeding real limits fails in CI rather than on-chain.

```rust
use lafiya_test_utils::budgeted_env;

#[test]
fn transfer_within_budget() {
    let env = budgeted_env();
    // ... exercise the contract ...
    // Optional: assert a per-test ceiling (see below).
    env.cost_estimate().budget().print();
}
```

If a test genuinely needs a different ceiling (e.g. a deliberately expensive
migration), pass an override rather than calling `reset_unlimited`:

```rust
let env = lafiya_test_utils::budgeted_env_with_limits(cpu, mem);
```

## Per-test CPU and memory in JUnit output

The test harness reports each test's CPU instruction count and memory bytes in
the JUnit XML so cost trends are visible in CI. The reporter reads the budget
consumed at the end of each test (via the `budgeted_env()` helper, which
records the final `cost_estimate()` into a thread-local that the reporter
drains) and emits them as properties on the `<testcase>` element:

```xml
<testcase name="transfer_within_budget" classname="..." time="0.003">
  <properties>
    <property name="cpu_instructions" value="48213"/>
    <property name="memory_bytes" value="10240"/>
  </properties>
</testcase>
```

Tests that do not use `budgeted_env()` simply omit these properties; the CI
job warns (but does not fail) when a contract test lacks them, so migrations
can proceed incrementally. To assert a realistic per-test ceiling directly in
a test, use the helper's assertion:

```rust
lafiya_test_utils::assert_within_budget(&env, /* cpu */ 100_000, /* mem */ 65_536);
```

Ceilings should be set from observed cost plus headroom, not guessed, and
tightened over time as regressions are fixed.