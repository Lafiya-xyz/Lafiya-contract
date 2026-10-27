# Storage Cost Benchmarks

This document tracks the resource-regression guard for the attester allowlist as it scales.

## Enforced checkpoints

`large_attester_allowlist_load` records the budget of `add_attester` after the
allowlist reaches each checkpoint and fails if either ceiling is exceeded.

| Operation | Attesters | Maximum CPU instructions | Maximum memory bytes |
|---|---:|---:|---:|
| `add_attester` | 10 | 2,000,000 | 1,000,000 |
| `add_attester` | 100 | 2,000,000 | 1,000,000 |
| `add_attester` | 1,000 | 2,000,000 | 1,000,000 |

## Methodology

The test initializes `attester-registry`, adds 1,000 distinct addresses, and
reads `Budget::cpu_instruction_cost` and `Budget::memory_bytes_cost` immediately
after additions 10, 100, and 1,000. Soroban resets metering before each
top-level invocation, so each checkpoint measures one `add_attester` call with
the allowlist at that size. This catches a size-dependent scan without conflating
the result with the cost of populating the preceding entries.

These are native-test regression measurements, not fee estimates. Native
registration omits Wasm execution, transaction-envelope processing, and some
ledger work, so the values must not be used to predict production fees.

Run the test and print its measured values with:

```sh
cargo test -p attester-registry large_attester_allowlist_load -- --nocapture
```

Last source verification: contract commit `82f7473`, with `soroban-sdk 27.0.0`
from the workspace lockfile. Update this reference and review the ceilings
whenever contract behavior or the pinned SDK cost model changes.

## Attestation rate limiting

`attestation-registry` can cap how many attestations each attester records
per window (`set_attestation_rate_limit(max_per_window, window_ledgers)`),
with per-attester overrides for high-volume clinical sites
(`set_attester_rate_limit` / `remove_attester_rate_limit`). This bounds the
blast radius of a compromised or fraudulent attester key: it limits how many
persistent attestation entries (and their rent) one key can create before it
is caught, and how many must be revoked afterwards.

### Storage

| Key | Storage | Lifetime |
|---|---|---|
| `RateLimit` | instance | Until changed by the admin |
| `RateLimitOverride(attester)` | persistent | Extended to 90 days on write |
| `RateWindow(attester)` → `{ window_start_ledger, count }` | temporary | TTL set to the remaining ledgers in the window |

The window counter uses **temporary** storage. It is cheaper than persistent
storage, it expires on its own when the window ends so nothing needs cleaning
up, and it is never restored from archival. If the entry expires before its
window ends, the next `attest` finds no window and starts a fresh one, so the
limiter **fails open**. That is acceptable for rate limiting. At worst an
attester gets one extra window's worth of attestations, and the allowlist and
revocation remain the real controls. Windows are capped at 30 days of
ledgers (`InvalidRateLimit` otherwise) so the entry's TTL always fits within
the network's maximum entry TTL.

### Budget

Measured with `env.cost_estimate().budget()` on the second `attest` by the
same attester (in-process `soroban-sdk` 27 test host):

| `attest` | CPU instructions | Memory bytes |
|---|---|---|
| Rate limiting disabled (default) | 268,515 | 107,366 |
| Rate limiting enabled | 362,594 | 139,568 |
| Added cost | +94,079 (~35%) | +32,202 |

The added cost is one instance read of `RateLimit`, one persistent read of the
override, one temporary read and write of `RateWindow`, and one TTL extension.
With rate limiting disabled, the only extra work is one read of `RateLimit`
from the already loaded instance storage.

### Default limits

Rate limiting is **off** until the admin configures it, so an upgrade does not
change existing behaviour. No pilot or load-campaign data exists yet. Until it
does, the recommended starting configuration is:

```text
set_attestation_rate_limit(max_per_window = 100, window_ledgers = 17_280)  # 100 per attester per ~24h
```

A CHW verifying records by hand in the field is unlikely to exceed a few dozen
attestations a day. 100 per day leaves headroom for busy clinic days, while
capping a compromised key at about 100 bad entries per day instead of
thousands per hour. Give clinical sites with known higher volume an override
rather than raising the global limit. Revisit these numbers once pilot data
is available.

### Watchdog integration

`RateLimitHit { attester, retry_after_ledger }` is published on the
attestation that **fills** an attester's window, so it appears at most once
per attester per window and cannot be used to spam. Attempts after that fail
with `RateLimited` (error `10`). Failed invocations publish no events, so the
watchdog alerts on the event and not on failures. Call
`get_rate_limit_retry_after(attester)` to get the ledger at which the
attester may attest again.

Recommended watchdog rules (for the event indexer described in
[`architecture/event-indexing.md`](architecture/event-indexing.md)):

- **Warn** on any `RateLimitHit`: an attester reached its daily cap.
- **Page** when the same attester hits the limit in two consecutive windows,
  or when three or more distinct attesters hit it within one window. Both
  suggest key compromise or scripted incentive farming. Consider
  `suspend_attester` on `attester-registry` and review that attester's
  recent `AttestationRecorded` events.
- **Audit** every `RateLimitSet` event, since it changes the limit for all attesters.

## Attestation TTL extension and rent at scale

`attestation-registry` stores each attestation as a persistent entry keyed by
`record_hash`. Persistent entries archive when their TTL runs out, so a proof
that must stay verifiable has to be extended before expiry. Anyone can call
`extend_attestation_ttl(record_hash, extend_to)`; the call is permissionless
and does not require the original attester.

### Bounded sponsor model

`extend_attestation_ttl` is deliberately narrow so an open caller cannot
abuse it:

- It only **extends**. If `extend_to` is at or below the entry's current
  `live_until_ledger`, the call is a no-op (or fails with `TtlNotExtended`);
  TTL can never be shortened.
- It never writes attestation data. The stored record, attester, timestamp,
  and revocation flag are untouched; only the entry's TTL is bumped.
- `extend_to` is capped to `current_ledger + MAX_TTL_EXTENSION_LEDGERS`
  (the network's maximum entry TTL). Requests beyond the cap are clamped, so
  no single call can pin an entry indefinitely.
- The caller pays the rent for the extension, so a sponsor (a clinic, an
  indexer, or a patient's wallet) funds longevity without gaining any write
  authority over the record.

Because the operation is idempotent and monotonic, a watchdog can call it on
every entry approaching expiry without coordinating with the attester.

### Rent per attestation at scale

Soroban persistent rent is charged per ledger the entry stays live, on the
entry's serialized size. An attestation entry is small and fixed-size (hash,
attester address, ledger timestamp, revocation flag), so rent scales linearly
with the number of live attestations and with the horizon chosen:

| Live attestations | Approx. entry bytes | Rent for 30 days | Rent for 90 days |
|---:|---:|---:|---:|
| 1,000 | ~120 | baseline | 3x baseline |
| 100,000 | ~120 | 100x baseline | 300x baseline |
| 1,000,000 | ~120 | 1,000x baseline | 3,000x baseline |

The absolute stroop figures depend on the pinned `soroban-sdk` cost model and
the network's rent parameters, so treat the table as a scaling guide rather
than a fee quote. The important property is that per-entry rent is constant:
there is no size-dependent scan, so extending one entry costs the same whether
the registry holds a thousand or a million attestations. Budget for 1M live
attestations by multiplying the measured single-entry extension cost by
1,000,000 and adding the storage footprint above.

### Restoration of archived entries

When an entry has already archived, `extend_attestation_ttl` cannot revive it
in place. Restore it first, then extend:

1. Detect the archive: a read of the entry returns `ArchivedEntry` (or the
   indexer reports the entry as archived).
2. Restore with `RestoreFootprintOp` for the entry's ledger key (the
   `record_hash` persistent key). This is a standard Soroban operation and
   requires the entry's footprint, not its data.
3. Re-read the entry to confirm it is live and unchanged.
4. Call `extend_attestation_ttl(record_hash, extend_to)` to set the desired
   horizon.

The CLI exposes this as a single helper:

```sh
attestation-cli restore --record-hash <hash> --extend-to <ledger>
```

`restore` submits the restore footprint, waits for confirmation, then calls
`extend_attestation_ttl`. It is safe to re-run: an already-live entry skips
the restore step and only extends if `extend_to` is beyond the current TTL.

### Watchdog guidance for approaching expiry

The event indexer should track each attestation's `live_until_ledger` (from
the entry's TTL, refreshed on every `extend_attestation_ttl`) and alert before
archival:

- **Warn** when an entry is within 7 days of expiry and no extension has been
  recorded.
- **Page** when an entry is within 1 day of expiry, or when it has archived
  and a verifier has requested it.
- **Auto-extend** (optional) for entries backing active patient cards, using
  the bounded sponsor model above so the extension stays within the cap.

## Status-list segments (bulk revocation)

`status-list` stores revocation state as versioned bitstring segments so
offline verifiers can sync a compact list instead of querying per record.

### Segment sizing at 1M entries

Each segment is a fixed-size bitstring plus a version and a commitment:

| Entries | Segment bits | Segment bytes (approx.) | Segments |
|---:|---:|---:|---:|
| 1,000 | 1,000 | ~160 | 1 |
| 100,000 | 100,000 | ~12,500 | 1 |
| 1,000,000 | 1,000,000 | ~125,000 | 1 |

A 1M-entry status list is roughly 125 KB of bitstring payload plus per-segment
metadata (version, commitment, root). Syncing it is a single segment fetch
rather than 1M per-record lookups.

### Sync time at 1M entries

Measured in the sandbox reference verifier against a local RPC:

| Step | Time |
|---|---|
| Fetch segment (1M bits) | ~40 ms |
| Verify commitment against signed root | ~5 ms |
| Decode bitstring | ~15 ms |
| Total cold sync | ~60 ms |
| Incremental sync (version bump) | ~10 ms |

These are sandbox measurements, not production guarantees; network latency
dominates in practice. The key result is that sync cost is independent of the
number of records and depends only on segment size, so a verifier can cache
the segment and re-validate cheaply on each version bump.

### Index correlation

Status-list indices are allocated so they do not reveal record identity: the
index is derived from a keyed hash of the record, not from the record hash
itself, and indices are assigned in allocation order rather than by any
record-derived ordering. See
[`adr/0012-w3c-vc-export-format.md`](adr/0012-w3c-vc-export-format.md) for
the threat analysis of index correlation.
