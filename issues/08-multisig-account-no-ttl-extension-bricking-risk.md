---
title: "[audit] SEC-02: multisig-account never extends its own storage TTL — archival permanently bricks the account"
labels: bug, priority:p0, security, architecture, severity:critical
---

**Severity:** Critical
**Difficulty:** N/A — occurs passively through ordinary low-frequency use; no attacker required
**Type:** Availability / Storage-Rent leading to Irrecoverable Denial of Service

> Same root cause class as ARCH-01 (attester-registry's missing TTL
> policy) but materially more severe: a registry losing a persistent
> entry is data loss; a **custom account** losing its instance storage is
> total, potentially permanent loss of the account's ability to authorize
> anything — including the transaction needed to fix it.

## Summary

`multisig-account` writes `Threshold` and every `Signer(BytesN<32>)`
entry to instance storage exactly once, in `__constructor`, and never
extends their TTL anywhere else in the contract. `__check_auth` only
reads this storage; reads do not extend TTL in Soroban. Per ADR-0003,
this contract is the team's intended production replacement for the
current single-admin model on both registries — an account that
authorizes infrequently is a realistic operating pattern, and Soroban's
state-archival mechanism means this account's own configuration can be
evicted through simple inactivity.

## Location

`contracts/multisig-account/src/lib.rs:41-57` (`__constructor`, writing
`Signer`/`Threshold` to instance storage); no `extend_ttl` call exists
anywhere else in the file (confirmed via `grep -n "extend_ttl"
contracts/multisig-account/src/lib.rs` → no matches).

## Technical Detail

```rust
41	    pub fn __constructor(env: Env, signers: Vec<BytesN<32>>, threshold: u32) {
42	        if threshold == 0 || threshold > signers.len() {
43	            panic_with_error!(&env, Error::InvalidThreshold);
44	        }
45	
46	        for signer in signers.iter() {
47	            let key = DataKey::Signer(signer);
48	            if env.storage().instance().has(&key) {
49	                panic_with_error!(&env, Error::DuplicateSigner);
50	            }
51	            env.storage().instance().set(&key, &());
52	        }
53	
54	        env.storage()
55	            .instance()
56	            .set(&DataKey::Threshold, &threshold);
57	    }
```

`__check_auth` (lines 65-106) only performs `env.storage().instance()
.get(&DataKey::Threshold)` and `.has(&DataKey::Signer(...))` — both reads.
Unlike `attestation-registry::attest()`, which at least bumps its own
instance TTL on every write, this contract has no write path after
construction and therefore no natural point where TTL gets extended at
all.

## Impact

If this account's instance storage TTL lapses (archived due to
infrequent use — e.g. an admin account that only transacts once a month
to add an attester, well within a plausible operating cadence), reads of
`Threshold`/`Signer` begin failing. Soroban supports state-archival
recovery via explicit restore operations within a recovery window, at a
cost — but that requires off-chain operational awareness and timely
action. If recovery is not performed within the window, or is not
operationally set up at all, the result is **permanent loss of the
account's ability to authorize any transaction**, including the
transaction that would restore or fix it, since any fix would itself
require authorizing a call through the now-inaccessible account. For an
account gating administrative control of both registries per ADR-0003,
this is a custody-ending failure mode, not merely a data-availability
incident — the difference between this and ARCH-01 is the difference
between "an attester needs re-adding" and "the admin key is gone."

## Recommendation

Requires a design decision — the right mechanism depends on constraints
this audit cannot fully verify (specifically, whether Soroban's execution
model permits storage-extending writes *during* the `__check_auth` phase,
which has historically had tighter restrictions than ordinary contract
invocations in some SDK versions):

1. **Extend TTL inside `__check_auth` itself**, if permitted by the
   current `soroban-sdk` version's auth-check execution semantics —
   verify this against the pinned SDK version before committing to this
   approach.
2. **A permissionless `keep_alive()` entrypoint** that does nothing but
   extend instance TTL, callable by anyone (or any signer) and driven by
   a scheduled off-chain job — simplest and most auditable, at the cost
   of depending on an off-chain scheduler staying alive as a backstop
   (the same tradeoff ARCH-01 weighs and rejects as a *sole* mechanism,
   but may be acceptable here specifically as a documented backstop given
   option 1's uncertainty).
3. **Operational runbook**, if neither in-contract mechanism is adopted:
   document the required TTL-extension cadence and ownership explicitly —
   the current silent default (no mechanism, no documented runbook) is
   the actual defect, not the absence of automation per se.

## Verification

- [ ] A maintainer decision is recorded on which mechanism is used.
- [ ] The chosen mechanism is implemented and tested, including a test
      that advances the ledger sequence past the instance TTL threshold
      and confirms the account remains authorizable.
- [ ] The chosen mechanism (or runbook) is documented in
      `docs/adr/0003-single-admin-initial-model.md`'s follow-up section or
      a new ADR — directly relevant to that ADR's stated gate on
      production/mainnet deployment.
