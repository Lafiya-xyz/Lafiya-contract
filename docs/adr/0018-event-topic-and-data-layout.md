# ADR-0018: Event topic and data layout for indexer filter efficiency

- Status: Accepted
- Related: [stability policy](../stability-policy.md), `catalog/events.json`,
  `.github/workflows/schema-impact.yml`

## Context

RPC `getEvents` filters only on topics, not on data fields. Any query key
that lives in event data forces the indexer to over-fetch every event of that
type and filter client-side.

## Query patterns vs. current layout

| Query | Events | Key location today | Filterable via RPC? |
| --- | --- | --- | --- |
| By record hash | `AttestationRecorded`, `AttestationWithdrawn`, `AttestationRevoked` | topic | yes |
| By attester | `AttesterAdded/Removed/Suspended/Reinstated/InfoUpdated`, `AttestationWithdrawn`, `RateLimitHit` | topic | yes |
| By attester | **`AttestationRecorded`** | **data** | **no** |
| By type | all | event-name prefix topic | yes |

The only high-volume event whose main secondary query key is not a topic is
`AttestationRecorded` (`attester` is in data). It is also the most frequent
event, so "all attestations by attester X" currently needs a scan of every
`AttestationRecorded`.

## Decision

1. **Do not change the layout of any existing event.** Per the stability
   policy, adding/removing/reordering `#[topic]` fields is a breaking change
   requiring two minor releases of dual emission. Moving `attester` into the
   topics of `AttestationRecorded` is the recommended future change, to be
   done as a dual-emit migration (new `AttestationRecordedV2` with topics
   `[record_hash, attester]`, old event kept for two minors), tracked
   separately and flagged by the schema-impact workflow.
2. **New events follow the rule "every field used as a lookup key is a
   topic; everything else is data".** Applied now to
   `AttesterRemovalScheduled` / `AttesterRemovalCancelled` (`attester`
   topic) and `AttestationCoverageRecorded` (`record_hash`, `attester`
   topics; version and bitmap in data).
3. Keep at most two lookup topics plus the name prefix per event, and keep
   high-cardinality, non-queried values (timestamps, counters, reasons) in
   data.

## Consequences

- Indexer fetch volume: for per-attester attestation queries, a topic filter
  returns only that attester's events instead of all `AttestationRecorded`
  events, so volume falls from O(all attestations) to O(attestations by that
  attester) once V2 ships. No reduction for existing events until then.
- Event size (estimate from XDR layout, not an on-chain measurement): moving
  one 40-byte address from a data map entry to a topic removes the map key
  (`attester` symbol, ~16 bytes padded) and map framing, about 20 bytes per
  event, with fee impact below the noise of the existing baseline.
  The measurement for `AttestationRecordedV2` must be recorded in
  `budgets/baseline.json` when it is implemented.
