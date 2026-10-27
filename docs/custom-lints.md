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

CI runtime: first run builds the lint crate and installs cargo-dylint (locally: about 1 min to build and test the lint crate, plus the lint pass over the workspace; cold CI time not yet measured); the cache in the `dylint` job removes most of it.
