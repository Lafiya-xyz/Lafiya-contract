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

`bindings/multisig-account` covers the contract's data types (`Signature`,
`Errors`) but, being an account contract, has no state-changing methods of
its own to call. Building and submitting a `__check_auth`-authorized call is
covered instead by [`@lafiya/multisig-auth`](../packages/multisig-auth), a
hand-maintained helper package (not generated) for computing the signature
payload and encoding N-of-M signatures in the shape the contract expects.
`scripts/conformance/check_bindings_drift.py` covers `multisig-account`'s
bindings the same way it does the other two contracts.

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
