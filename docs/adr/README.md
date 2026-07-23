# Architecture Decision Records

This directory records significant architectural decisions for `Lafiya-contract`.
An ADR explains the context behind a decision, the selected approach, the alternatives
considered, and the consequences contributors should preserve or revisit.

## Process

1. Copy [`0000-template.md`](0000-template.md).
2. Assign the next unused four-digit number and a short kebab-case filename.
3. Open the ADR as **Proposed** in the same pull request as the related change, or before
   implementation when early review would materially reduce rework.
4. Change the status to **Accepted** once maintainers approve the decision.
5. Do not rewrite an accepted ADR to change history. Add a new ADR and mark the old one
   **Superseded by ADR-NNNN** instead.

Small implementation details that do not constrain future work do not need an ADR.
Security boundaries, privacy assumptions, contract interfaces, storage models, and
administrative trust models generally do.

## Status values

- **Proposed** — under discussion and not yet binding.
- **Accepted** — the current architectural direction.
- **Deprecated** — retained for history but no longer recommended.
- **Superseded by ADR-NNNN** — replaced by a later decision.

## Index

| ADR | Decision | Status |
| --- | --- | --- |
| [ADR-0001](0001-hash-only-on-chain-footprint.md) | Keep health data off-chain and store only opaque commitments | Accepted |
| [ADR-0002](0002-contractclient-boundary.md) | Use a local `#[contractclient]` interface for registry-to-registry calls | Accepted |
| [ADR-0003](0003-single-admin-initial-model.md) | Use a single admin address for the pre-alpha contracts | Accepted |
| [ADR-0007](0007-unscoped-multisig-authorization.md) | Keep multisig authorization unscoped during pre-alpha | Proposed |
