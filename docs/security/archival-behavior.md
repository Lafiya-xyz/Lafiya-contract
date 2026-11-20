# Archival Behavior of Persistent Storage Keys

This document is the published outcome table for issue #723: a systematic
"archive each persistent key class and observe" test harness. It records, for
every persistent key class in every contract, what happens when the key is
archived (its TTL expires and the entry is evicted from the live ledger), and
the security rationale for the observed behavior.

The harness lives in the test suite (`archival_behavior` tests) and drives the
test environment's ledger manipulation to advance past the TTL of targeted
keys, then exercises the read paths that consume each key.

## Key classes and durability

Each contract declares its storage keys via `StorageKey::durability`. Only keys
whose durability is `Persistent` are subject to archival; `Temporary` keys are
expected to disappear and must never be relied upon for trust decisions.
`Instance` keys are archived only when the whole contract instance is archived
and are out of scope for per-key TTL expiry.

## Observed outcomes

Legend for the "Archived read" column:

- **`None`** — the read returns `Option::None` (indistinguishable from "never
  existed").
- **Traps** — the call panics / traps (e.g. `unwrap` on a missing entry, or an
  explicit `panic!`).
- **Restore required** — the call surfaces a restore requirement (returns an
  error or otherwise signals that the entry must be restored before use).

| Contract | Key class | Durability | Archived read | Security rationale |
| --- | --- | --- | --- | --- |
| _populated by the harness_ | | | | |

> **Status:** This table is populated by the archival-behavior harness. Each
> row is generated from the enumerated `StorageKey` variants and the observed
> behavior of the corresponding read path. Rows are added as contracts and key
> classes are covered; the table is not considered complete until every
> `Persistent` key class in every contract has a row.

## Invariant: archival must never increase trust

The central security invariant verified by the harness is:

> Archiving any persistent key must never make a record appear **more**
> trusted. In particular, archived revocation, dispute, or withdrawal markers
> must not cause a record to be treated as valid, active, or undisputed.

For each key class the harness asserts that the post-archival read path is
**fail-closed**: a missing marker is treated as "unknown / not proven" rather
than "absent, therefore trusted". Where a read returns `None`, the consuming
logic must distinguish "never existed" from "archived" or must default to the
less-trusted interpretation.

## Findings

Any case where archival makes a record appear more trusted is filed as a
security bug (private advisory per `SECURITY.md`) and listed here.

| Finding | Key class | Description | Status |
| --- | --- | --- | --- |
| _none filed_ | | | |

## Notes on synthetic data

All test fixtures use synthetic data only. No real health data, QR payloads, or
secrets are used in the harness or recorded in this document.