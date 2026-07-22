---
title: "[feature]: MultisigAccount::__check_auth ignores auth_contexts — signers authorize arbitrary invocations with no scoping"
labels: enhancement, priority:p2, security, architecture
---

> Architect-tier least-privilege question, not a bug in the sense of
> broken code — the signature says `_auth_contexts: Vec<Context>`, so the
> current behavior is deliberate, but it's worth surfacing as a
> conscious design decision given what this account is meant to custody.

## Description

`CustomAccountInterface::__check_auth`'s third parameter,
`_auth_contexts: Vec<Context>`, is prefixed with an underscore and never
read:

```rust
fn __check_auth(
    env: Env,
    signature_payload: Hash<32>,
    signatures: Self::Signature,
    _auth_contexts: Vec<Context>,
) -> Result<(), Error> {
```

Soroban passes `auth_contexts` specifically so a custom account *can*
inspect what it's being asked to authorize — which contract, which
function, which arguments, and any sub-invocations — and make a scoped
decision (e.g. "this account will sign for `attester-registry::
add_attester` and `attestation-registry::revoke_attestation`, but not for
transferring the native XLM balance, and not for calling arbitrary
third-party contracts"). This contract does none of that: once
`threshold`-many valid, known-signer, correctly-ordered signatures are
presented over the payload hash, `__check_auth` returns `Ok(())`
unconditionally, for **any** invocation the payload was constructed for —
sending funds, calling any contract, anything.

Per ADR-0003, this account is intended to become the `Admin` for both
`attester-registry` and `attestation-registry` — a narrow, specific
role. As deployed today, though, the account contract itself enforces no
scope: it's a general-purpose N-of-M signer, not an
admin-of-these-two-registries-specifically signer. That's not
automatically wrong (a general multisig is a reasonable, simpler
building block, and scope could be enforced entirely at the caller/policy
layer instead), but it does mean:

- If this account is ever funded directly with XLM (e.g. to pay for its
  own transaction fees, which is a normal operational pattern for
  Soroban accounts), the same N signers who can add an attester can also
  drain that balance to any address, with no separate spending policy.
- If the signer set is ever reused across other purposes beyond
  administering these two registries (tempting, since standing up a
  second multisig has real operational cost), there is no contract-level
  guardrail stopping it from being used to authorize something the
  signers didn't intend the *quorum* to cover — every signer is
  individually trusted for literally anything the account is asked to
  sign, with no per-purpose separation of duties.

## Proposed change

At minimum, this should be a documented decision rather than an implicit
default:

- If unscoped signing is intentional (simplicity, and scope enforcement
  belongs elsewhere — e.g. never fund this account beyond fee reserves,
  never reuse its signer set for anything else), say so explicitly in
  `docs/adr/0003-single-admin-initial-model.md`'s follow-up section or a
  new ADR, with the operational constraints that make it safe spelled
  out (e.g. "this account must never hold a balance beyond N XLM fee
  reserve" as a documented operational rule, since the contract itself
  won't enforce it).
- If scoping is wanted, `_auth_contexts` gives everything needed to
  implement it — e.g. reject contexts whose `Context::Contract`
  `contract` address isn't in an allowlisted set, or whose invoked
  function isn't in an allowlisted set per contract. This is real added
  complexity (mirrors the "custom N-of-M logic" tradeoff ADR-0003 already
  weighed once and deferred in favor of using account abstraction — this
  issue isn't proposing reopening that decision, just noting the
  in-contract scoping question is a layer above "multisig vs. single key,"
  and got skipped rather than decided).

## Alternatives considered

- **Enforce scope entirely off-chain** (only ever construct authorization
  entries for the intended two contracts via `lafiya-cli`/deploy tooling,
  never sign anything else). Workable as a stopgap, but relies on every
  signer's tooling behaving correctly forever — an on-chain guardrail is
  strictly stronger since it holds even if a signer's tooling is
  compromised or a signer is careless.

## Scope

- Component(s) affected: multisig-account
- Does this change a contract's public function signatures? No — this is
  about the *body* of `__check_auth`, not its interface.

## Acceptance criteria

- [ ] A maintainer decision recorded (scope-in-contract vs.
      scope-off-chain-with-documented-constraints) in an ADR.
- [ ] If scoping is chosen: implemented, tested (a test that a correctly
      threshold-signed authorization for an out-of-scope contract/function
      is rejected), and any allowlist made admin-configurable through the
      same signer quorum rather than hardcoded, if the team wants
      flexibility to administer additional contracts later.
- [ ] If off-chain-only is chosen: the operational constraints (funding
      limits, signer-set reuse policy) are written down somewhere a future
      operator will actually find them (ADR-0003 follow-up, or
      `docs/releasing.md`/a new ops doc).

## Additional context

- [ADR-0003](../docs/adr/0003-single-admin-initial-model.md)
- `contracts/multisig-account/src/lib.rs`
