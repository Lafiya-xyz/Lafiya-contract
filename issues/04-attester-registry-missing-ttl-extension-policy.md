---
title: "[audit] ARCH-01: attester-registry never calls extend_ttl — declared rent policy is dead code"
labels: enhancement, priority:p1, architecture, severity:high
---

**Severity:** High
**Difficulty:** Low — occurs passively through normal inactivity, no attacker required
**Type:** Availability / Storage-Rent (Soroban state-archival denial of service)

> Depends on BUILD-01 landing first (the file must compile before this
> can be implemented and tested). Related to SEC-02 (multisig-account has
> the same gap, with more severe blast radius).

## Summary

`attester-registry` declares a TTL/rent-extension policy as constants but
never calls `extend_ttl` anywhere in the file. Persistent `Attester`
entries and instance storage (`Admin`, `PendingAdmin`, `SchemaVersion`)
are never rent-bumped, so they are subject to Soroban's state-archival
mechanism: once their TTL lapses, reads fail or return empty,
indistinguishable on-chain from deliberate removal.

## Location

`contracts/attester-registry/src/lib.rs:37-38` (policy constants,
unused); contrast with `contracts/attestation-registry/src/lib.rs:174-176`
(the sibling contract's partial implementation of the same policy).

## Technical Detail

```rust
34	/// Instance storage TTL policy:
35	/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
36	/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
37	const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
38	const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;
```

These constants are declared and never referenced again in the file — no
`env.storage().instance().extend_ttl(...)` call exists anywhere in
`attester-registry/src/lib.rs`, and no `env.storage().persistent()
.extend_ttl(...)` call exists for the `Attester(Address)` /
`Suspended(Address)` entries `add_attester`, `suspend_attester`, etc.
write. Commit `43f7b2e` ("Fix #14: Add instance storage TTL extensions to
Admin and AttesterRegistry") introduced these constants with a title
implying this contract was covered; the implementation was not carried
through.

The sibling contract, `attestation-registry`, does call
`env.storage().instance().extend_ttl(INSTANCE_LIFETIME_THRESHOLD,
INSTANCE_BUMP_AMOUNT)` in `attest()` (lines 174-176) — but only for its
own instance storage, never for the specific `Attestation(record_hash)`
persistent entry it just wrote. So the gap exists in both contracts, in
different shapes: total absence in `attester-registry`, partial coverage
in `attestation-registry`.

## Impact

A `get`/`has` read (which `is_attester`, `get_attester_info`, and
`get_attestation` all perform) does **not** extend TTL in Soroban. An
attester added once and never touched by another admin action will,
after the network's archival period elapses, have its persistent entry
evicted. `is_attester` then returns `false` and `get_attester_info`
returns `None` — on-chain state indistinguishable from the attester
having been deliberately removed. The same applies to `Admin`/
`PendingAdmin`/`SchemaVersion` on instance storage. For a registry whose
entire purpose is being a durable allowlist, silent data loss through
inactivity is a correctness failure with security implications
(a legitimately allowlisted attester could stop being recognized with no
admin action taken).

## Recommendation

- Call `env.storage().instance().extend_ttl(INSTANCE_LIFETIME_THRESHOLD,
  INSTANCE_BUMP_AMOUNT)` in every state-mutating function in
  `attester-registry` (`initialize`, `add_attester`/
  `add_attester_with_info`, `remove_attester`, `suspend_attester`,
  `reinstate_attester`, `propose_admin`, `accept_admin`), matching the
  pattern already present in `attestation-registry::attest()`.
- Extend the TTL of the specific persistent entry written by
  `add_attester`/`add_attester_with_info`. Decide explicitly — not by
  default — whether read paths (`is_attester`, `get_attester_info`)
  should also bump TTL, given they are likely the highest-frequency
  operation against this contract.
- In `attestation-registry::attest()`, additionally extend the TTL of the
  specific `Attestation(record_hash)` entry being written, not just
  instance storage.
- Document the agreed policy — a short ADR or an addition to
  `docs/architecture/storage-versioning.md`, which currently covers
  schema evolution but not rent — so this doesn't drift silently again.

## Alternatives Considered

**Off-chain keep-alive transactions from the indexer/CLI as the sole
mechanism.** Rejected: makes on-chain data availability dependent on an
off-chain service staying up indefinitely, undermining the
"independently checkable, on-chain trust anchor" property
`docs/adr/0001-hash-only-on-chain-footprint.md` is built around. Usable
as a backstop, not a substitute for in-contract extension.

## Verification

- [ ] Every state-mutating function in both contracts extends the
      relevant instance and/or persistent TTL.
- [ ] A test advancing the ledger sequence past
      `INSTANCE_LIFETIME_THRESHOLD` confirms a previously-added
      attester/attestation remains readable, added to both contracts'
      `src/test.rs`.
- [ ] The agreed TTL/rent policy is documented (new ADR or an addition to
      `docs/architecture/storage-versioning.md`).
