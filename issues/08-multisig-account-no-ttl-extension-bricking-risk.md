---
title: "[bug]: multisig-account never extends its own instance storage TTL — expiry permanently bricks the account"
labels: bug, priority:p1, security, architecture
---

> Related to #04 (attester-registry's missing TTL policy) but materially
> more severe: a registry losing a persistent entry to expiry is data
> loss; a **custom account** losing its instance storage to expiry is
> total, permanent loss of the account's ability to authorize anything.

## Description

`contracts/multisig-account/src/lib.rs` stores `Threshold` and every
`Signer(BytesN<32>)` entry in **instance** storage, written once in
`__constructor` and never touched again by any other function in the
contract:

```rust
pub fn __constructor(env: Env, signers: Vec<BytesN<32>>, threshold: u32) {
    ...
    for signer in signers.iter() {
        let key = DataKey::Signer(signer);
        ...
        env.storage().instance().set(&key, &());
    }
    env.storage().instance().set(&DataKey::Threshold, &threshold);
}
```

`__check_auth` only **reads** this storage (`env.storage().instance()
.get(&DataKey::Threshold)`, `.has(&DataKey::Signer(...))`) — reads do not
extend TTL in Soroban. There is no `extend_ttl` call anywhere in this
file, unlike `attestation-registry::attest()` which at least bumps its
own instance TTL on every write.

Per ADR-0003, this multisig contract is the team's intended production
replacement for the current single-admin model — the address configured
as `Admin` on both registries is meant to eventually *be* a deployed
`MultisigAccount`. If this contract's instance storage TTL expires (gets
archived because the account happened to go unused past the rent
threshold — plausible for an admin account that only transacts
occasionally, e.g. adding an attester once a month), the archived entry
means `Threshold`/`Signer` reads start failing. Depending on how the
account is restored (Soroban supports state archival recovery via
explicit restore operations, at a cost, if the entry hasn't been evicted
past a recovery window), this is at minimum an availability incident
requiring off-chain intervention, and at worst — if restoration isn't
performed in time or isn't operationally set up — **permanent loss of the
ability to authorize any transaction from this account**, including the
transactions that would be needed to fix the problem, since fixing it
would itself require authorizing a call through the now-broken account.
For an account gating the admin of both registries, that's a
custody-ending failure mode, not just a data-availability one.

This is a more severe instance of the same root problem as the
attester-registry TTL issue (#04), but distinct enough — different
contract, different blast radius (bricked custody vs. missing allowlist
entries), different fix shape (an account contract can't easily "bump TTL
on every admin call" the way a registry can, since `__check_auth` is
called by the protocol, not invoked as a regular contract call the
account itself controls the body of in the same way) — to warrant tracking
separately rather than folding into #04.

## Expected behavior

The account's instance storage TTL is kept alive indefinitely through
routine operation, or through an explicit, documented keep-alive
mechanism, so an admin account that authorizes transactions infrequently
doesn't risk archival.

## Actual behavior

No TTL extension exists anywhere in the contract. An infrequently-used
account is a real archival risk.

## Proposed fix (needs design, not just a one-line patch)

A few shapes worth evaluating together with a maintainer, since the right
answer depends on constraints this audit can't fully see (how
`__check_auth` interacts with TTL extension calls made *during*
authorization, and whether that's even permitted/metered the same way in
Soroban's auth-check execution context):

1. **Extend TTL inside `__check_auth` itself**, if Soroban's execution
   model permits storage-extending writes during the auth-check phase (this
   needs verifying against current `soroban-sdk` semantics — auth checks
   historically have had tighter restrictions than normal contract
   invocations in some SDK versions).
2. **A permissionless `keep_alive()` entrypoint** any signer (or even
   anyone) can call, that does nothing but extend instance TTL, callable
   as a scheduled off-chain cron job (the `lafiya-cli`/deploy tooling
   already has a network-config-aware CLI that could host this) — simplest
   and most auditable, at the cost of depending on an off-chain scheduler
   staying alive, which is the same tradeoff #04 already weighs and
   rejects as a *sole* mechanism but which may be acceptable as a
   documented backstop here.
3. **Operational runbook**: document the required TTL-extension cadence
   and who's responsible, if the team decides an in-contract mechanism
   isn't worth the complexity for now — but that decision should be
   explicit, not the current silent default.

## Acceptance criteria

- [ ] A maintainer decision recorded on which mechanism is used.
- [ ] Whichever mechanism is chosen is implemented and tested, including
      a test that advances the ledger sequence past the instance TTL
      threshold and confirms the account remains authorizable.
- [ ] The chosen mechanism (or the operational runbook, if that's the
      route) is documented in `docs/adr/0003-single-admin-initial-model.md`'s
      follow-up section or a new short ADR, since this is directly
      relevant to that ADR's "Before a production or mainnet deployment"
      gate.

## Environment

- Contract(s) affected: multisig-account
- Verified by reading source; `cargo` unavailable in this audit
  environment.
