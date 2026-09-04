# lafiya-commitment

Prototype implementation for the record-commitment canonicalization spike
([issue #133](https://github.com/Lafiya-xyz/Lafiya-contract/issues/133)).
The full decision — comparison matrix, threat analysis, and migration plan —
is in [`docs/adr/0008-record-commitment-canonicalization.md`](../../docs/adr/0008-record-commitment-canonicalization.md).
This crate implements the Lafiya Record Commitment v1 (LRC-1) scheme that
ADR describes: a fixed-schema, length-prefixed binary encoding hashed with
SHA-256 under a domain-separation tag and an explicit version byte.

This is off-chain application code, not a Soroban contract. Per
[ADR-0001](../../docs/adr/0001-hash-only-on-chain-footprint.md), the
contracts only ever see the resulting opaque 32-byte commitment.

## Layout

- [`src/lib.rs`](src/lib.rs) — Rust implementation.
- [`reference-ts/lrc1.ts`](reference-ts/lrc1.ts) — TypeScript reference
  implementation, field-for-field and byte-for-byte identical to the Rust
  version. Illustrative of the construction the web application should
  follow; not a published package.
- [`vectors/lrc1-test-vectors.json`](vectors/lrc1-test-vectors.json) — the
  shared fixture both implementations are tested against.

## Running the tests

Rust:

```sh
cargo test -p lafiya-commitment
```

TypeScript (Node 22+; no build step or dependency — Node strips the type
annotations natively):

```sh
node crates/lafiya-commitment/reference-ts/lrc1.test.ts
```

Both suites load `vectors/lrc1-test-vectors.json` and assert their output
matches it byte-for-byte, which is the cross-language determinism the spike
was asked to demonstrate.
