---
title: "[audit] ARCH-03: attestation-registry has no path to repoint AttesterRegistry after initialize"
labels: enhancement, priority:p2, architecture, severity:medium
---

**Severity:** Medium
**Difficulty:** N/A — a missing capability, not an active exploit
**Type:** Operational Resilience / Missing Recovery Path

> A gap in the upgrade/recovery story, not a defect in current behavior.
> Worth a deliberate decision rather than an unexamined default,
> especially since ADR-0003 already anticipates the admin side of this
> evolving.

## Summary

`attestation-registry` stores which `attester-registry` contract it
trusts once, at `initialize`, with no function anywhere in the contract
to change it afterward. Every other piece of configurable state in this
codebase has a defined lifecycle (admin transfer, wasm upgrade); this
address does not.

## Location

`contracts/attestation-registry/src/lib.rs:95-108` (`initialize`, storing
`DataKey::AttesterRegistry` at line 103, never written again anywhere in
the file).

## Technical Detail

- `Admin` has a full two-step transfer flow: `propose_admin`/
  `accept_admin` (lines 111-145).
- `attester-registry` itself has `upgrade(new_wasm_hash)` (lines 256-262)
  for in-place wasm upgrades at a fixed `Address`.
- `attestation-registry`'s pointer to *which* `attester-registry` address
  it trusts (`DataKey::AttesterRegistry`) has none of this — it is
  read-only after `initialize`.

`docs/adr/0002-contractclient-boundary.md` designed the cross-contract
call boundary to be swappable at the *signature* level ("contributors
must update the local interface" when the callee's function signature
changes) but never addressed the callee's *address* changing at runtime.

## Impact

If `attester-registry` ever needs replacement rather than in-place
upgrade — a new deployment with a different `Address`, migration to a
different allowlist implementation, or disaster recovery after a lost
upgrade key on the existing instance — `attestation-registry` has no
supported recovery path. The only options are redeploying
`attestation-registry` itself, which discards all attestation history at
that contract address (directly contradicting the "tamper-evident trust
anchor" property `docs/adr/0001-hash-only-on-chain-footprint.md` is built
around), or an unreviewed, ad hoc emergency patch.

## Recommendation

Add an admin-gated `set_attester_registry(new_registry: Address)`.
Whether it needs the same two-step propose/accept ceremony as admin
transfer (a wrong address here silently breaks every future `attest()`
call — mitigated post-BUILD-01 by `attest()` failing closed via
`Error::InvalidRegistryWiring` rather than silently allowlisting
everyone, but still an availability incident until caught) or a
single-step admin-gated call is sufficient is a judgment call about
expected frequency and blast radius of a mistake — worth a short explicit
discussion rather than defaulting to whichever is less code. Emit
`AttesterRegistryRepointed { previous, new }` for indexer/audit
visibility, consistent with the existing `AdminTransferred` pattern.

## Alternatives Considered

**No in-contract mechanism; treat replacement as full redeployment of
both contracts.** Valid if that is the team's accepted operational model
— but it should be written down (this issue, or an addition to
ADR-0002's Follow-up section) as a decision, not left as an unexamined
gap.

## Scope

Adds one new function; does not change existing signatures or the
attestation schema. Still worth flagging to `lafiya-web`/`lafiya-cli`
maintainers as new admin-key attack surface (an address the admin can now
change).

## Verification

- [ ] Decision recorded — either the function is added, or "redeploy is
      the path" is explicitly documented. Either outcome closes this
      issue.
- [ ] If implemented: admin-gated, tested for authorized and
      unauthorized-caller paths per `CONTRIBUTING.md`'s testing
      requirements, with an event emitted.
