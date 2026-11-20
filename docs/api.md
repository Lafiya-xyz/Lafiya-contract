# API reference (rustdoc)

The rustdoc for this version is published alongside the site:

- [`attester_registry`](api/attester_registry/index.html)
- [`attestation_registry`](api/attestation_registry/index.html)
- [`incentive_pool`](api/incentive_pool/index.html)
- [`multisig_account`](api/multisig_account/index.html)
- [`lafiya_commitment`](api/lafiya_commitment/index.html)
- [`lafiya_config`](api/lafiya_config/index.html)
- [`lafiya_rpc_resilience`](api/lafiya_rpc_resilience/index.html)

Build it locally with `cargo doc --workspace --no-deps --open`.

## Contract configuration

Contracts are configured atomically at deploy time through their `__constructor`
entry point; there is no externally callable `initialize` function. For
`incentive_pool`, the constructor takes the admin, approver, token,
`attester_registry`, and the remaining configuration values, validates them
(including interface probes of the supplied `attester_registry` and `token`),
and stores them before the contract becomes reachable. This closes the
deploy-then-initialize front-running window in which an attacker could call
`initialize` first and set themselves as admin and approver.

> **Breaking change:** the previous public `initialize(admin, approver, token,
> attester_registry, …)` entry point has been removed. Deploy tooling, CLI
> commands, bindings, and sandbox scenarios must pass configuration to the
> constructor at deploy time instead of issuing a follow-up `initialize`
> transaction.