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
4. **`check_bindings_drift.py`** -- regenerates the TypeScript client with
   the same `stellar contract bindings typescript` command `make bindings`
   uses, and diffs its function/error surface against the committed
   `bindings/<contract>/src/index.ts`.

All four read the *compiled* `.wasm` artifact (via
`stellar contract info interface`), not the Rust source -- so they catch
drift introduced anywhere between source and the thing that actually gets
deployed (a merge that drops a function body, a stale generated client,
etc.), not just Rust-level signature changes.

## Running it

```bash
make conformance          # build contracts, run all four checks
make conformance-update   # regenerate snapshots + docs/events.md after a deliberate interface change
```

Requires the `stellar` CLI on `PATH` (same prerequisite as `make bindings`;
see `docs/typescript-bindings.md`).

`check_bindings_drift.py` does not have an `--update` mode -- if it finds
drift, regenerate the affected bindings the normal way (`make bindings`)
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
| `check_bindings_drift.py` | TS binding regeneration diff |
| `snapshots/*.json` | Committed golden interface snapshots |
