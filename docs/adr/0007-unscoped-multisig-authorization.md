# ADR-0007: Keep multisig authorization unscoped during pre-alpha

- **Status:** Proposed
- **Date:** 2026-07-23
- **Deciders:** Lafiya contract maintainers

## Context

ADR-0003 selected a Soroban account contract as the preferred successor to a single
administrator key. The resulting `multisig-account` contract verifies an N-of-M set of
ed25519 signatures in `__check_auth`.

Soroban also supplies `auth_contexts` to `__check_auth`. Those contexts describe the
contracts, functions, arguments, and nested invocations being authorized. The current
implementation intentionally does not inspect them. A valid signer quorum can therefore
authorize any invocation represented by the signed payload; the account is a general-purpose
multisig, not an account whose authority is limited to Lafiya's two registry contracts.

Adding an on-chain allowlist would narrow that authority, but it would also introduce new
security-sensitive state and policy questions:

- which registry addresses and functions are allowed;
- how the policy is initialized, updated, and recovered;
- whether contract upgrades and admin transfers are allowed;
- how nested invocations are evaluated; and
- whether policy changes require the same quorum they are intended to constrain.

Hardcoding pre-alpha contract addresses or function names would make redeployment and upgrades
brittle. Adding configurable policy without first specifying its lifecycle would replace a
visible operational risk with an under-designed on-chain control.

## Decision

During pre-alpha, keep `multisig-account` authorization unscoped. Threshold signatures prove
that a quorum approved the complete Soroban authorization payload, but the contract does not
restrict the target contract, function, arguments, asset movement, or nested invocations.

This is an explicit interim trust model, not a claim of least-privilege enforcement. Operators
must apply all of these controls:

1. Use a signer set dedicated to Lafiya registry administration. Do not reuse any signer key or
   the same quorum for treasury, personal, validator, or unrelated application authority.
2. Do not use the multisig address as a treasury or payment account. Keep only a documented,
   bounded XLM fee reserve needed for near-term administration, and sweep any excess.
3. Require every signer to inspect the decoded authorization tree, including contract address,
   function, arguments, asset movements, and sub-invocations, using independently obtained
   expected registry addresses. A payload hash or transaction label alone is insufficient.
4. Treat signer tooling as part of the security boundary. Use independently maintained tooling
   or an out-of-band review for quorum approval; one compromised interface must not be the only
   representation all signers inspect.
5. Do not deploy this account as a production or mainnet administrator until maintainers either
   accept these residual risks for that environment or replace this ADR with a scoped policy.

The deployment record must state the dedicated signer-set identifier, threshold, approved
registry addresses, fee-reserve ceiling, and the procedure for reviewing and sweeping the
balance. Secret keys must never appear in that record.

## Consequences

### Positive

- The existing contract interface, storage layout, and tested signature behavior remain stable.
- The authority boundary and signer responsibilities are explicit to reviewers and operators.
- Pre-alpha redeployments are not coupled to hardcoded addresses or an incomplete policy-update
  mechanism.

### Trade-offs and risks

- A valid quorum can authorize calls to any contract and can transfer any assets held by the
  multisig address.
- Contract code cannot protect against colluding signers, compromised signing tools, policy
  misunderstanding, signer-set reuse, or an excessive account balance.
- Safety depends on operational controls that are weaker than an on-chain allowlist.
- The multisig must not be described as registry-scoped or least-privileged.

## Alternatives considered

### Allowlist registry contract addresses and functions in `auth_contexts`

Deferred until the policy lifecycle, nested-invocation rules, upgrade behavior, and recovery
path are specified together. This remains the preferred stronger control for production.

### Hardcode the current registry addresses or function names

Rejected because addresses change on redeployment and the administrative interface can evolve.
An immutable list could lock operators out of legitimate recovery or migration operations.

### Rely on signing tooling without documenting constraints

Rejected. Off-chain enforcement is only a conscious architecture decision when its assumptions,
limits, and operator duties are reviewable.

## Follow-up

- Before production or mainnet, propose an ADR for configurable invocation scoping or explicitly
  accept the residual unscoped-authority risk for that environment.
- If scoping is implemented, test rejection of out-of-scope contract functions, asset transfers,
  and disallowed nested invocations.
- Add the deployment-record fields and independent authorization inspection to the deployment
  runbook tracked in issue #48.

## References

- [ADR-0003: Use a single admin address for the pre-alpha contracts](0003-single-admin-initial-model.md)
- [SEC-03 audit finding](https://github.com/Lafiya-xyz/Lafiya-contract/issues/109)
- [`multisig-account::__check_auth`](../../contracts/multisig-account/src/lib.rs)
