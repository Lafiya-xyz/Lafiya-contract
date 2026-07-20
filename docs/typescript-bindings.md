# TypeScript Client Bindings

To allow the patient-facing web application (`lafiya-web`) to interact with the deployed Soroban smart contracts, TypeScript client bindings are generated directly from the built contract WebAssembly (`.wasm`) files.

## Generation

The bindings are generated using the `stellar-cli` tool. A target is provided in the `Makefile` to automate this:

```bash
make bindings
```

This runs:
1. `make wasm` to build both contracts to `target/wasm32v1-none/release/`.
2. `stellar contract bindings typescript` to output the generated TS clients to:
   - `bindings/attester-registry`
   - `bindings/attestation-registry`

## Publishing & Consumption Strategy

After evaluating options for distributing the generated client bindings to `lafiya-web`, the following hybrid strategy was selected:

1. **Committed Bindings Directory (Primary):**
   - The generated client code in the `bindings/` directory is committed directly to the `lafiya-contract` repository.
   - This ensures that contract and client binding changes are always version-locked and tracked in source control together.
   - `lafiya-web` can consume these bindings via:
     - A git submodule pointing to this repository.
     - A direct Git dependency in `lafiya-web`'s `package.json` (e.g., `"@lafiya/contracts": "git+https://github.com/Lafiya-xyz/Lafiya-contract.git#semver:^0.1.0"`).
     - Standard workspace/monorepo references if they are brought into a monorepo setup in the future.

2. **NPM Registry Publishing (Secondary / Optional CI):**
   - If public/private package registry distribution is desired, a GitHub Action can be configured to pack and publish the generated `bindings/` to the `@lafiya` organization on the npm registry whenever a release tag is pushed.
