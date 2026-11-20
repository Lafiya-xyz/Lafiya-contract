# TypeScript Client Bindings

To allow the patient-facing web application (`lafiya-web`) to interact with the deployed Soroban smart contracts, TypeScript client bindings are generated directly from the built contract WebAssembly (`.wasm`) files.

## Generation

The bindings are generated using the `stellar-cli` tool. A target is provided in the `Makefile` to automate this:

```bash
make bindings
```

This runs:
1. `make wasm` to build the contracts to `target/wasm32v1-none/release/`.
2. `stellar contract bindings typescript` to output the generated TS clients to:
   - `bindings/attester-registry`
   - `bindings/attestation-registry`
   - `bindings/multisig-account`
   - `bindings/incentive-pool`

`bindings/multisig-account` covers the contract's data types (`Signature`,
`Errors`) but, being an account contract, has no state-changing methods of
its own to call. Building and submitting a `__check_auth`-authorized call is
covered instead by [`@lafiya/multisig-auth`](../packages/multisig-auth), a
hand-maintained helper package (not generated) for computing the signature
payload and encoding N-of-M signatures in the shape the contract expects.
`bindings/incentive-pool` covers the pool's views (e.g. pool state, balances,
and reward accounting) and its events, so `lafiya-web` can both read pool state
and decode pool events emitted on-chain.

`scripts/conformance/check_bindings_drift.py` covers `multisig-account`'s and
`incentive-pool`'s bindings the same way it does the other two contracts. The
drift gate runs in CI and fails when a contract's interface changes without the
corresponding `bindings/` being regenerated and committed.

## ScVal Round-Trip Fuzz Suite

Source-level drift checks catch interface changes, but not value-level encoding
mismatches (e.g. `i128`/`u64` BigInt handling, `Option<Vec<…>>`, enum tuple
variants, or map key ordering). The round-trip fuzz suite exercises every
`#[contracttype]` across the language boundary: Rust encodes random values to
XDR base64 plus JSON, the TypeScript bindings decode them, re-encode, and the
bytes must match exactly. The reverse direction (TS encodes, Rust decodes and
re-encodes) is checked the same way.

### Generated type list

The list of types under test is generated from the contract specs rather than
maintained by hand, so a newly added `#[contracttype]` is covered automatically:

```bash
make scval-types
```

This runs `scripts/conformance/gen_scval_types.py`, which reads the built
`.wasm` specs (the same inputs as `make bindings`) and writes
`scripts/conformance/scval_types.json` — one entry per contracttype per
contract, including nested types referenced by struct fields and enum payloads.
The generated file is committed; the drift gate fails if it is stale relative to
the specs.

### Generating fixtures

Fixtures are random values serialized by the Rust side:

```bash
make scval-fixtures            # default: 1000 values per type
make scval-fixtures COUNT=100  # smaller run for local iteration
```

This runs the Rust generator (`cargo test -p scval-fuzz -- --ignored` under the
hood), which uses `proptest`/`arbitrary` to produce values for every entry in
`scval_types.json` and writes them to
`scripts/conformance/fixtures/scval/<contract>/<type>.jsonl`, one
`{"xdr": "<base64>", "json": <value>}` record per line. The default of **1000
random values per type** is the guarantee enforced in CI; the generator fails if
any type ends up with fewer records.

### Running locally

Rust side (generates fixtures and verifies Rust→XDR→Rust round-trips):

```bash
cargo test -p scval-fuzz -- --ignored
```

TypeScript side (decodes fixtures with the bindings, re-encodes, asserts byte
equality, then runs the TS→Rust direction):

```bash
cd bindings && npm test -- scval-roundtrip
```

Run both after regenerating bindings or changing a contracttype. Regenerate the
type list and fixtures (`make scval-types scval-fixtures`) whenever a contract's
types change; the drift gate will flag a stale `scval_types.json` or fixture set.

### CI

The suite runs in CI on any change to `contracts/`, `bindings/`, or
`scripts/conformance/`, so encoding regressions surface before release.

## Publishing & Consumption Strategy

See [`PUBLISHING.md`](../PUBLISHING.md) for the canonical, up-to-date strategy. Summary:

1. **Committed Bindings Directory (Primary, in effect today):**
   - The generated client code in the `bindings/` directory is committed directly to the `lafiya-contract` repository.
   - This ensures that contract and client binding changes are always version-locked and tracked in source control together.
   - `lafiya-web` can consume these bindings via:
     - A git submodule pointing to this repository.
     - A direct Git dependency in `lafiya-web`'s `package.json` (e.g., `"@lafiya/contracts": "git+https://github.com/Lafiya-xyz/Lafiya-contract.git#semver:^0.1.0"`).
     - Standard workspace/monorepo references if they are brought into a monorepo setup in the future.

2. **NPM Registry Publishing (Secondary, planned, not yet live):**
   - Once `bindings/*/package.json` are scoped under `@lafiya` and given a `publishConfig`, a GitHub Action can pack and publish the generated `bindings/` to the npm registry whenever a release tag is pushed, coordinated with [ADR-0010](adr/0010-release-manifest-and-compatibility.md)'s release manifest.
