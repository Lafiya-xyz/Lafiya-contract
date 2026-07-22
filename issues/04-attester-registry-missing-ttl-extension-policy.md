---
title: "[feature]: attester-registry never extends storage TTL — allowlist entries can silently expire"
labels: enhancement, priority:p1, architecture
---

> Architect-tier: this is a storage-rent/liveness design gap, not a
> syntax bug. Depends on #02 (the file doesn't currently compile) landing
> first so this can be implemented and tested against a working baseline.

## Problem

Soroban's persistent and instance storage are rent-based: entries that
aren't touched (read via a TTL-extending op, or explicitly bumped) for
long enough get archived and reads start failing/returning empty, even
though the data was never explicitly deleted. Both registry contracts
define the same TTL policy constants:

```rust
// Instance storage TTL policy:
// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;
```

`attestation-registry::attest()` actually calls
`env.storage().instance().extend_ttl(INSTANCE_LIFETIME_THRESHOLD,
INSTANCE_BUMP_AMOUNT)`. But `attester-registry` declares the identical
constants and **never calls `extend_ttl` anywhere in the file** — not on
instance storage, and not on the `persistent` `Attester(Address)` /
`Suspended(Address)` entries that `add_attester`, `suspend_attester`, etc.
write. (Confirmed by reading the full file — commit history that added
these constants is `43f7b2e`, "Fix #14: Add instance storage TTL
extensions to Admin and AttesterRegistry," which per its own title
intended to cover this contract too.)

Consequence: an allowlisted attester added once and never touched again
(no admin action, no lookups that extend TTL — note `is_attester`/
`get_attester_info` are read-only `get`/`has` calls, which in Soroban do
*not* extend TTL by themselves) will eventually have their persistent
`Attester` entry archived by the network. After that, `is_attester`
returns `false` and `get_attester_info` returns `None` for a real,
never-removed attester — indistinguishable on-chain from having been
deliberately removed. Same failure mode for `Admin`/`PendingAdmin`/
`SchemaVersion` on instance storage, which also never get bumped in this
contract.

This is also somewhat separate from `attestation-registry`'s own gap: it
bumps *instance* TTL on every `attest()` call, but never touches the TTL
of the *persistent* `Attestation(record_hash)` entry it just wrote —
so an attestation that's written once and never re-attested can still
expire independently of the contract's instance storage staying alive.

## Proposed change

- In `attester-registry`, call `env.storage().instance().extend_ttl(...)`
  in every state-mutating admin function (at minimum `initialize`,
  `add_attester`/`add_attester_with_info`, `remove_attester`,
  `suspend_attester`, `reinstate_attester`, `propose_admin`,
  `accept_admin`), matching what `attestation-registry::attest()` already
  does for its own instance storage.
- Additionally extend the TTL of the specific `persistent` entry being
  written on `add_attester`/`add_attester_with_info` (and consider whether
  reads via `is_attester`/`get_attester_info` should also bump it, given
  those are likely the highest-frequency operation against this
  contract — that's a call the team should make explicitly rather than by
  omission, since bumping-on-read has its own cost tradeoffs).
- In `attestation-registry::attest()`, additionally extend the TTL of the
  specific `Attestation(record_hash)` persistent entry being written, not
  just the contract's instance storage.
- Consider whether `get_attestation`/`is_attester`/`get_attester_info`
  (read-only, callable by anyone, including from other contracts) should
  extend TTL on read — this is the kind of design question `docs/
  architecture/storage-versioning.md` doesn't currently answer for rent,
  only for schema evolution, so this issue may be a natural trigger for a
  short ADR of its own (`docs/adr/000X-storage-rent-policy.md`) covering
  both contracts consistently, rather than each contract growing its own
  ad hoc TTL logic.

## Alternatives considered

- **Rely on external "keep-alive" transactions from the off-chain
  indexer/CLI.** Rejected as the primary mechanism (though it can be a
  belt-and-suspenders backstop) because it makes the smart contract's own
  data availability dependent on an off-chain service staying up
  indefinitely, which undermines the "independently checkable, on-chain
  trust anchor" property `docs/adr/0001-hash-only-on-chain-footprint.md`
  is built around.

## Scope

- Component(s) affected: attester-registry, attestation-registry
- Does this change a contract's public function signatures or the
  attestation schema? No — this is internal storage-lifetime handling,
  not a signature or schema change. Existing callers are unaffected.

## Acceptance criteria

- [ ] Every state-mutating function in both contracts extends the
      relevant instance and/or persistent TTL.
- [ ] A test that advances the ledger sequence past
      `INSTANCE_LIFETIME_THRESHOLD` (Soroban's test `Env` supports
      manipulating the ledger sequence) and confirms a previously-added
      attester/attestation is still readable, added to both contracts'
      `src/test.rs`.
- [ ] A short ADR or an addition to `docs/architecture/storage-versioning.md`
      documenting the agreed TTL/rent policy so it doesn't silently drift
      again the way it did here.

## Additional context

Related: `docs/storage-cost.md` benchmarks CPU cost by allowlist size but
doesn't currently track storage-rent/TTL-bump cost, which would be worth
adding once this lands.
