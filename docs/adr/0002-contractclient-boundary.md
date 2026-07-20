# ADR-0002: Use a local `#[contractclient]` interface for registry-to-registry calls

- **Status:** Accepted
- **Date:** 2026-07-18
- **Deciders:** Lafiya contract maintainers

## Context

`attestation-registry` must ask `attester-registry` whether an address is currently allowlisted
before accepting an attestation. The call needs a typed Soroban client, but the release build
should not link the callee's contract implementation into the caller's Wasm artifact.

A direct runtime dependency on the full `attester-registry` crate would pull in code that the
caller does not execute locally, increase coupling and artifact size, and can produce linker
warnings when both contract implementations export functions with the same names, such as
`initialize`.

## Decision

`attestation-registry` will define a local `#[contractclient]` trait containing only the external
functions it calls from `attester-registry`. The current boundary is:

```rust
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
}
```

Production code will construct the generated client using the configured contract address and
call through this narrow interface.

A development-only crate dependency may be used to register the real callee contract in tests.
That dependency must remain outside the release dependency graph.

When a called signature changes, contributors must update the local interface and integration
tests together. The callee's exported contract interface remains the source of truth.

## Alternatives considered

### Depend directly on the full `attester-registry` crate at runtime

Rejected because it couples the caller to the callee implementation, can link unnecessary
contract code into the Wasm artifact, and has already exposed colliding-export warnings on the
pinned Soroban toolchain.

### Use untyped `Env::invoke_contract` calls

Rejected because manual symbol and argument construction weakens compile-time checking and makes
interface drift easier to miss.

### Duplicate the allowlist in `attestation-registry`

Rejected because two writable sources of truth could diverge and would complicate administration
and auditing.

## Consequences

### Positive

- The cross-contract dependency is explicit, typed, and limited to the required capability.
- Release Wasm avoids linking the callee implementation.
- `attester-registry` remains the single source of truth for allowlisting.
- Tests can still exercise the real cross-contract path.

### Trade-offs and risks

- The local trait duplicates the called function signature and must be kept synchronized.
- Interface drift is detected through compilation and tests rather than a shared runtime crate.
- Additional called functions require deliberate expansion of this boundary.

## Follow-up

- Keep a cross-contract integration test for every function added to the local client interface.
- Revisit the boundary if Soroban introduces a stable interface-artifact workflow that preserves
  the same Wasm isolation with less duplication.

## References

- [`contracts/attestation-registry/src/lib.rs`](../../contracts/attestation-registry/src/lib.rs)
- [`contracts/attestation-registry/Cargo.toml`](../../contracts/attestation-registry/Cargo.toml)
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md), cross-contract call guideline
