# ADR-0003: Use a single admin address for the pre-alpha contracts

- **Status:** Accepted
- **Date:** 2026-07-18
- **Deciders:** Lafiya contract maintainers

## Context

Both registry contracts need a principal that can configure security-sensitive state. The
current milestone is pre-alpha on Stellar testnet, where keeping the authorization surface small
helps the team validate the attestation flow before introducing production governance.

A single externally controlled key is also a single point of failure. Compromise or loss could
affect the attester allowlist, and the current contracts do not provide an in-contract rotation
or recovery path after initialization.

Soroban's `Address` abstraction can represent a user account or a contract account. This makes it
possible to evolve the administrator into a multisig/smart-account contract without embedding
custom threshold-signature logic in each registry.

## Decision

For the pre-alpha contracts, each registry stores one `Admin` address during `initialize`.
Admin-gated functions retrieve that address and call `require_auth()`.

This is an initial implementation simplification, not the intended production custody model.
Before a production or mainnet deployment, Lafiya must complete the multisig work tracked in
[issue #19](https://github.com/Lafiya-xyz/Lafiya-contract/issues/19). The preferred direction is
to initialize the registries with a Soroban multisig or smart-account contract that satisfies
standard authorization, rather than adding bespoke N-of-M logic to each registry.

Until that migration is validated, operators must treat the configured admin credential as a
high-value security secret and must not present the testnet governance model as production-ready.

## Alternatives considered

### Implement custom N-of-M authorization inside both registries now

Deferred because it would duplicate complex security-critical logic, expand the audit surface,
and slow validation of the core attestation milestone. Soroban account abstraction should be
investigated first.

### Use separate role addresses for every administrative function

Deferred until operational roles and production governance requirements are clearer. Premature
role fragmentation would add configuration and testing complexity without yet having a concrete
operator model.

### Remove administrative control after deployment

Rejected because the attester allowlist must be maintained and incorrect or compromised
attesters must be removable.

## Consequences

### Positive

- The initial authorization model is small, understandable, and easy to test.
- Admin-gated behavior consistently uses Soroban's standard `require_auth()` mechanism.
- The stored `Address` leaves a path to contract-account and multisig authorization.

### Trade-offs and risks

- A single user-controlled admin key is a custody and availability risk.
- There is no threshold approval, separation of duties, or built-in recovery in the current
  contracts.
- Production deployment is blocked on defining and validating a stronger admin setup.

## Follow-up

- Complete [issue #19](https://github.com/Lafiya-xyz/Lafiya-contract/issues/19).
- Add a test proving that a multisig-backed contract address can authorize admin-gated calls.
- Document deployment and recovery procedures for the selected production admin account.

## References

- [Issue #19: Add admin multisig / N-of-M authorization](https://github.com/Lafiya-xyz/Lafiya-contract/issues/19)
- [`contracts/attester-registry/src/lib.rs`](../../contracts/attester-registry/src/lib.rs)
- [`contracts/attestation-registry/src/lib.rs`](../../contracts/attestation-registry/src/lib.rs)
