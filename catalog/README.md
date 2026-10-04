# Error and event catalogs

**Generated files; do not hand-edit.** Regenerate with `make conformance-update` (or
`python3 scripts/conformance/gen_catalog.py`).

- `errors.json`: every contract error, keyed by `(contract, code)`. Codes overlap between
  contracts, so always decode with the contract kind.
- `events.json`: every contract event with its topics and data fields.

Codes and event shapes come from the interface snapshots pinned to the built Wasm. `doc`
and metadata come from the `///` comments on each `Error` variant, which can carry tags:
`@severity user|operator|bug` (default `user`), `@retryable true` (default `false`),
`@since <version>`, and `@deprecated <reason>`.

The TypeScript bindings export the same data from `@lafiya/<contract>/catalog`
(`CONTRACT_ERRORS` and `decodeContractError(contractKind, code)`). The release manifest
records each catalog's path and sha256. CI (`docs-checks`) fails if the catalogs are stale.
