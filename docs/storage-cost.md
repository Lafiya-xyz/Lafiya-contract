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
