---
title: "[audit] SEC-03: MultisigAccount::__check_auth ignores auth_contexts — no invocation scoping"
labels: enhancement, priority:p2, security, architecture, severity:medium
---

**Severity:** Medium
**Difficulty:** Low — requires no special access; any signer collusion, or a single compromised signer's tooling, exercises the full unscoped authority
**Type:** Access Control / Least-Privilege Violation (CWE-284)

> Not broken code in the sense of a bug — the parameter is deliberately
> named `_auth_contexts` — but a conscious design gap worth surfacing
> given what this account is intended to custody per ADR-0003.

## Summary

`__check_auth`'s third parameter, `auth_contexts: Vec<Context>`, carries
exactly the information Soroban gives a custom account to make a scoped
authorization decision — which contract, which function, which
arguments, which sub-invocations are being requested. This implementation
never reads it. Once `threshold`-many valid, known, correctly-ordered
signatures are presented over the payload hash, the account authorizes
**any** invocation the payload was constructed for, with no restriction
on target contract or function.

## Location

`contracts/multisig-account/src/lib.rs:65-70` (unused parameter),
`96-106` (unconditional `Ok(())` return with no scope check performed
anywhere in the function body).

## Technical Detail

```rust
65	    fn __check_auth(
66	        env: Env,
67	        signature_payload: Hash<32>,
68	        signatures: Self::Signature,
69	        _auth_contexts: Vec<Context>,
70	    ) -> Result<(), Error> {
```

The leading underscore is the Rust convention for "intentionally unused"
— confirming this is a deliberate simplification, not an oversight, but
one that should be a documented decision given the account's intended
role.

## Impact

Per ADR-0003, this account is intended to become `Admin` for both
`attester-registry` and `attestation-registry` — a narrow role. As
implemented, the contract itself enforces no such narrowing: it is a
general-purpose N-of-M signer, not an
admin-of-these-two-registries-specifically signer.

- If the account is ever funded directly with XLM (a normal operational
  pattern, e.g. to cover its own transaction fees), the same signer
  quorum that can add an attester can authorize draining that balance to
  any address — there is no separate spending policy.
- If the signer set is ever reused for any purpose beyond administering
  these two registries, there is no contract-level guardrail preventing
  the quorum from authorizing something outside what signers understood
  themselves to be approving when they agreed to be signers. Every signer
  is individually trusted for anything the account is asked to sign, with
  no per-purpose separation of duties.

## Recommendation

Requires an explicit decision, not a default:

- **If unscoped signing is intentional:** document it — in
  `docs/adr/0003-single-admin-initial-model.md`'s follow-up section or a
  new ADR — along with the operational constraints that make it safe
  (e.g. "this account must never hold a balance beyond an N-XLM fee
  reserve," "this signer set must not be reused for any purpose outside
  registry administration"), since the contract will not enforce these
  itself.
- **If scoping is wanted:** `auth_contexts` provides everything needed —
  reject any `Context::Contract` entry whose `contract` address or
  invoked function is outside an allowlisted set. This adds real
  complexity and is a separate design question from "multisig vs. single
  key" (which ADR-0003 already resolved) — it is a layer above that
  decision and was skipped rather than explicitly deferred.

## Alternatives Considered

**Enforce scope entirely off-chain** (only ever construct authorization
entries for the two intended contracts via `lafiya-cli`/deploy tooling).
Workable as an interim measure, but depends on every signer's tooling
behaving correctly indefinitely. An on-chain guardrail is strictly
stronger: it holds even if a signer's tooling is compromised or a signer
is careless.

## Verification

- [ ] A maintainer decision is recorded (in-contract scoping vs.
      documented off-chain constraints) via ADR.
- [ ] If scoping is chosen: implemented, tested (a threshold-signed
      authorization for an out-of-scope contract/function is rejected),
      with any allowlist configurable through the existing signer quorum
      rather than hardcoded.
- [ ] If off-chain-only is chosen: the operational constraints (funding
      limits, signer-set reuse policy) are written down where a future
      operator will find them.
