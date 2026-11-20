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
| `non_try_cross_contract_call` | Panicking cross-contract client calls trap with a host error when the callee is archived, mid-upgrade, or misconfigured, instead of returning a typed contract error. | `registry.is_attester(&attester)` | `registry.try_is_attester(&attester).map_err(\|_\| Error::AttesterRegistryUnavailable)?` |

## `non_try_cross_contract_call`

Contract crates generate a client for every imported contract interface. The
generated client exposes both panicking methods (`is_attester`) and fallible
`try_` variants (`try_is_attester`). The panicking methods trap on any
transport or contract failure — the caller cannot map the failure to a typed
error, and the whole transaction aborts with an opaque host error.

This lint flags any call to a generated cross-contract client method that is
not a `try_` variant, in contract crates (`contracts/*/src`). It covers all
five contracts, so a new cross-contract call cannot reintroduce the trap
pattern.

Bad — traps if the registry is unavailable:

```rust
let is_attester = registry.is_attester(&attester);
if !is_attester {
    return Err(Error::NotAttester);
}
```

Good — maps the failure to a typed contract error:

```rust
let is_attester = registry
    .try_is_attester(&attester)
    .map_err(|_| Error::AttesterRegistryUnavailable)?
    .map_err(|_| Error::AttesterRegistryUnavailable)?;
if !is_attester {
    return Err(Error::NotAttester);
}
```

Note the two `map_err` layers: the outer `Result` is the transport/host
failure, the inner `Result` is the callee's own contract error. Both must be
mapped to the typed error so callers can distinguish "registry unavailable"
from a genuine negative answer.

CI wiring: the lint runs in the existing `dylint` job alongside the other
lints (`cargo dylint --path lints --all -- --workspace --all-targets`), so no
separate job is needed. The job's cache covers the lint crate build; the lint
pass over the workspace adds negligible time.

CI runtime: first run builds the lint crate and installs cargo-dylint (locally: about 1 min to build and test the lint crate, plus the lint pass over the workspace; cold CI time not yet measured); the cache in the `dylint` job removes most of it.