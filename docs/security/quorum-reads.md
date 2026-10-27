# Quorum Reads

A single RPC provider can lie about attester status or attestation existence
and cause a false "verified" verdict. Security-sensitive reads (`verify`:
attestation lookup, history, attester allowlist status) can instead be
cross-checked across independent providers.

Implementations: `lafiya_rpc_resilience::quorum` (Rust) and
`QuorumChainReader` in `@lafiya/verifier-sdk` (TypeScript).

## Policy

- `k`: minimum number of agreeing, fresh providers (default 2).
- `maxLedgerSkew`: providers further behind the newest reported ledger are
  excluded as stale (ledger alignment).
- Rust only: `stop_at_k` stops querying once `k` agree.

## Outcomes

| Situation | Result |
| --- | --- |
| `k` or more fresh providers agree and none contradict | Value returned |
| Fresh providers return different values | Disagreement: `QuorumError("disagreement")`, verdict `unavailable` (never "verified") |
| Too few fresh answers (failures, staleness) | Insufficient: `QuorumError("insufficient")`, verdict `unavailable` |

## Metrics

Both implementations expose reads, disagreements, stale providers excluded,
provider failures, insufficient-quorum counts, and last read latency. Alert on
any non-zero disagreement rate.

## Latency

Providers are queried in parallel in the TypeScript reader, so overhead is the
slowest provider plus one latest-ledger call. Measure it via `lastLatencyMs`.
Tune it with `k`, fewer providers, or (Rust) `stop_at_k`, which trades away
detection of a liar among providers not yet asked.

## Trust assumptions

- Fewer than `k` providers are malicious or colluding.
- Providers are independently operated (different operators and
  infrastructure); two endpoints of one vendor count as one.
- A provider can inflate its reported ledger to make honest providers look
  stale; the effect is denial of verification, not a false positive.
- Any disagreement between fresh providers is treated as a fault, even when `k`
  agree, trading availability for safety.
- Quorum protects against dishonest RPC, not against compromise of the
  contracts or trust anchors themselves.
