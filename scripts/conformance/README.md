# Contract Interface Conformance Tooling (prototype)

Spike prototype for [#137](https://github.com/Lafiya-xyz/Lafiya-contract/issues/137).
See [`docs/spikes/0137-contract-interface-conformance.md`](../../docs/spikes/0137-contract-interface-conformance.md)
for the full write-up, approach comparison, and findings; this file just
covers how to run the tooling.

## What it checks

```
make conformance
```

runs four independent checks against the built contract `.wasm` files in
`target/wasm32v1-none/release/`:

1. **`check_snapshot.py`** -- diffs each contract's extracted interface
   (functions, error enum, events, shared types) against a committed golden
   snapshot in `snapshots/<contract>.json`. Fails on any signature, error
   code, or event schema change that wasn't accompanied by a snapshot
   update.
2. **`check_error_docs.py`** -- cross-checks the `Error` enum baked into
   the Wasm against the tables in `docs/error-codes.md`.
3. **`gen_events_doc.py --check`** -- verifies `docs/events.md` (the
   generated event schema reference) is still in sync with the Wasm.
4. **`cargo xtask generate --check`** -- the single golden-snapshot gate
   for every generated artifact: interface snapshots, TypeScript bindings
   (`bindings/<contract>/src/index.ts`), error/event catalogs,
   `docs/events.md`, the event decoders, the indexer OpenAPI operation
   index (`golden/openapi-operations.txt`) and the release manifest
   (`golden/release-manifest.json`, build-specific fields masked). It
   regenerates everything, restores the committed files, and prints a
   semantic diff (one line per changed JSON key path, changed lines for
   text) for anything that drifted.

5. **`gen_events_ts.py --check`** -- verifies the generated TypeScript
   event decoders in `packages/events/src/generated.ts` (`@lafiya/events`)
   match the event entries in the committed snapshots. It reads the
   snapshots rather than the Wasm, so it also runs in CI without the
   `stellar` CLI (the `events-package` job).

The first four read the *compiled* `.wasm` artifact (via
`stellar contract info interface`), not the Rust source -- so they catch
drift introduced anywhere between source and the thing that actually gets
deployed (a merge that drops a function body, a stale generated client,
etc.), not just Rust-level signature changes.

## Running it

```bash
make conformance          # build contracts, run all checks
make conformance-update   # `cargo xtask generate`: rewrite every generated artifact after a deliberate change
```

These checks run on every pull request in the `conformance` job of
`.github/workflows/ci.yml`, using the `stellar` CLI version pinned there
(`STELLAR_CLI_VERSION`).

Requires the `stellar` CLI on `PATH` (same prerequisite as `make bindings`;
see `docs/typescript-bindings.md`).

If `cargo xtask generate --check` reports drift, run `cargo xtask generate`
and commit the result, so the diff gets human review.

## Scope

The snapshot/docs/events checks cover `attester-registry` and
`attestation-registry` (see `contracts.py`); add an entry there to extend
coverage to `multisig-account` or a future contract.

## Files

| File | Purpose |
|---|---|
| `contracts.py` | Registry of contracts + their Wasm/bindings paths |
| `extract_interface.py` | Wraps `stellar contract info interface`, normalizes + sorts the spec |
| `spec_types.py` | Renders spec type nodes (e.g. `{"option": ...}`) as readable strings |
| `check_snapshot.py` | Interface snapshot diff/update |
| `check_error_docs.py` | Error code <-> `docs/error-codes.md` cross-check |
| `gen_events_doc.py` | Generates/checks `docs/events.md` |
| `snapshots/*.json` | Committed golden interface snapshots |
