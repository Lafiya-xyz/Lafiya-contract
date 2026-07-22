---
title: "[feature]: attestation-registry has no way to repoint AttesterRegistry after initialize"
labels: enhancement, priority:p2, architecture
---

> Architect-tier: a gap in the upgrade/recovery story, not a bug in
> current behavior. Lower priority than #01-#03 (build/CI) and #04-#05,
> but worth deciding deliberately rather than by omission, especially
> since ADR-0003 already anticipates the admin side of this evolving.

## Problem

`attestation-registry::initialize(admin, attester_registry)` stores the
`attester_registry` address once, in instance storage, under
`DataKey::AttesterRegistry`. There is no function anywhere in the
contract that ever updates that stored address afterward — every other
piece of configurable state has a lifecycle:

- `Admin` has a full two-step transfer flow (`propose_admin`/
  `accept_admin`).
- `attester-registry` itself has an `upgrade(new_wasm_hash)` entrypoint
  for in-place wasm upgrades (same contract `Address`, new code).

But `attestation-registry`'s pointer to *which* `attester-registry`
contract it trusts is fixed forever at `initialize` time. If
`attester-registry` ever needs to be replaced rather than upgraded
in-place — a new deployment with a different `Address`, a migration to a
different allowlist implementation, disaster recovery after a lost
upgrade key on the old instance — `attestation-registry` has no
supported path to follow it. The only options are redeploying
`attestation-registry` itself (losing all attestation history at that
address, contradicting the "tamper-evident trust anchor" property
`docs/adr/0001-hash-only-on-chain-footprint.md` is built around) or an ad
hoc emergency patch nobody has designed or reviewed.

`docs/adr/0002-contractclient-boundary.md` explicitly designed the
cross-contract call boundary to be swappable in principle ("A
development-only crate dependency may be used to register the real
callee contract in tests... When a called signature changes,
contributors must update the local interface") but only covers the
*signature* changing, not the *address* changing at runtime.

## Proposed change

Add an admin-gated `set_attester_registry(new_registry: Address)` (naming
to match the existing `propose_admin`/`accept_admin` two-step pattern if
the team wants the same blast-radius protection for this as for admin
transfer — a wrong address here silently breaks every future `attest()`
call until noticed, with `attest()` currently failing closed via the
compile-error-adjacent `InvalidRegistryWiring` error path once #02 is
fixed, so at least it fails loudly rather than silently allowlisting
everyone). Emit an event (e.g. `AttesterRegistryRepointed { previous,
new }`) so the off-chain indexer and any auditor can see the change,
consistent with how `AdminTransferred` is already handled.

Whether this needs the same two-step propose/accept ceremony as admin
transfer, or a single-step admin-gated call is sufficient, is a judgment
call about how often this is expected to happen and how bad a mistake
would be — worth a short discussion rather than defaulting to whichever
is less code.

## Alternatives considered

- **Do nothing; treat `attester-registry` replacement as "redeploy
  everything."** Valid if the team's actual operational model is that a
  full redeployment (both contracts, fresh addresses, migrated off-chain
  indexer cursor) is the accepted recovery path for this scenario. If so,
  that should be written down (this doc, or a short addition to
  `docs/adr/0002-contractclient-boundary.md`'s "Follow-up" section) so
  it's a decision, not an unexamined gap.

## Scope

- Component(s) affected: attestation-registry
- Does this change a contract's public function signatures? Adds one new
  function; does not change existing signatures or the attestation
  schema. Still worth a heads-up to `lafiya-web`/`lafiya-cli` maintainers
  since it's new attack surface (an address the admin can now change) any
  security review of the admin key would want to know about.

## Acceptance criteria

- [ ] Decision recorded (new function added, or explicit "redeploy is the
      path" documented) — either outcome closes this issue as long as
      it's a documented decision.
- [ ] If implemented: admin-gated, tested for both the authorized and
      unauthorized-caller paths per `CONTRIBUTING.md`'s testing
      requirements, and an event emitted.

## Additional context

- [ADR-0002](../docs/adr/0002-contractclient-boundary.md)
- [ADR-0003](../docs/adr/0003-single-admin-initial-model.md) — the
  existing two-step admin transfer this issue's proposed
  `set_attester_registry` would likely want to mirror stylistically.
