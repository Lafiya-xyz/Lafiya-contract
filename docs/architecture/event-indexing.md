# Event Indexing Design Spec

## Overview

Lafiya contracts currently declare the following on-chain event schemas:

- `AdminTransferred` (`attester-registry`, `attestation-registry`, `incentive-pool`)
- `Initialized` (`attester-registry`, `incentive-pool`)
- `AttesterAdded`
- `AttesterInfoUpdated` (`attester-registry`)
- `AttesterRemoved`
- `AttesterRevoked`
- `AttesterRotated` (`attester-registry`) — emitted when an attester rotates its key while retaining enrollment
- `AttesterSuspended`
- `AttesterReinstated`
- `AttestationRecorded`
- `AttestationBatchAnchored` (`attestation-registry`)
- `AttestationRevoked`
- `AttestationWithdrawn`
- `RecordVersionLinked`
- `Upgraded` (`attester-registry`, `attestation-registry`)
- `Paused` (`attester-registry`, `attestation-registry`, `incentive-pool`)
- `Unpaused` (`attester-registry`, `attestation-registry`, `incentive-pool`)
- `AttesterRegistryRepointed` (`attestation-registry`, `incentive-pool`)
- `PoolFunded` (`incentive-pool`)
- `PoolWithdrawn` (`incentive-pool`)
- `WorkItemApproved` (`incentive-pool`)
- `PayoutClaimed` (`incentive-pool`)
- `RateLimitHit` (`attestation-registry`) — published at most once per attester per rate-limit window, on the attestation that fills the window
- `RateLimitSet` (`attestation-registry`)
- `AdminTransferProposed`, `AdminTransferCancelled` (`attester-registry`, `attestation-registry`, `incentive-pool`)
- `MultisigConfigured`, `AuthorizationApproved` (`multisig-account`)

`Initialized` is emitted by `attester-registry` from its deployment constructor.
`MultisigConfigured` is emitted during multisig deployment, and
`AuthorizationApproved` records the timestamp and signer keys for each successful
authorization. Indexers should consume these events alongside registry activity.

Every registry event payload includes `contract_kind` and `schema_version`.
Registry event schema version `2` adds these fields; consumers should branch on
both values rather than infer contract type from event name or contract address.
`AttestationRecorded` also includes its per-record `sequence` and optional
`evicted_sequence`, allowing an indexer to mirror the bounded FIFO history.
`AttestationRevoked` includes the authorizing admin, the number of entries
removed, and a short reason symbol. Reason symbols are public and must not
contain personal or other sensitive data.

These events need to be consumed by the off‑chain services used by **lafiya‑web** to display the verified status in near‑real‑time. This document outlines the design of an **event indexing / webhook service** that polls or streams Soroban events and reconciles them with the existing Supabase‑backed profile data.

## Architecture Options

### 1. Polling (Periodic RPC Calls)
- **Mechanism**: Use the Soroban RPC `getEvents` endpoint periodically (e.g., every 30 seconds) to fetch new events since the last known ledger sequence.
- **Pros**:
  - Simple to implement; no long‑running connections.
  - Works with any RPC node, even those without event‑streaming support.
- **Cons**:
  - Latency bounded by poll interval.
  - Potential for missed events if the node lags or rate‑limits.
  - Increased load on RPC nodes under high frequency.

### 2. RPC Event Streaming (WebSocket / SSE)
- **Mechanism**: Connect to a Soroban RPC node that provides a continuous event stream (WebSocket or Server‑Sent Events). The service maintains the cursor (ledger sequence & offset) and processes events as they arrive.
- **Pros**:
  - Near‑instantaneous delivery (< 1 s).
  - Guarantees ordering and no gaps when the cursor is persisted.
- **Cons**:
  - Requires a node that supports streaming; not all public RPC providers expose it.
  - Needs reconnection logic and back‑pressure handling.

## Recommended Approach

Given the need for **low latency** and **reliability**, we recommend a **hybrid approach**:
1. **Primary**: Use RPC event streaming when a compatible node is configured (e.g., a dedicated Soroban‑node with WebSocket support).
2. **Fallback**: Switch to polling when the stream connection drops or when the node does not support streaming. The fallback poll interval should be short (≈ 15 s) to minimise latency.

The service will persist the **cursor** (last processed ledger & offset) in Supabase. On restart it resumes from the stored cursor, ensuring no events are missed.

## Integration with Existing Supabase Profile Data

1. **Event Processor**
   - Receives events, parses the relevant fields (e.g., attester address, subject address, timestamp).
   - Writes a row to a new `event_log` table in Supabase for auditability.
2. **Profile Updater**
   - For `AttestationRecorded`, update the `profiles` table (e.g., set `verified = true`, store attestation metadata).
   - For `AttestationRevoked`, remove the indexed attestation and set the corresponding profile's `verified` state to false.
   - For `AttesterAdded`, add the account in a secondary `attesters` table; for `AttesterRemoved` / `AttesterRevoked`, remove it while retaining its historical status transitions.
   - For `AttesterSuspended` / `AttesterReinstated`, update the account's active status without losing its allowlist history.
   - Record `AdminTransferred` as a contract-administration audit event; it does not directly change profile verification state.
   - If a future contract release begins publishing `Initialized`, record it as an administration audit event as well.
   - For `PoolFunded`, record donor funding events for the incentive pool audit trail.
   - For `PoolWithdrawn`, record recovery/withdrawal events.
   - For `WorkItemApproved`, record work-item approval events linking attesters to specific items.
   - For `PayoutClaimed`, update the CHW incentive ledger and deduct from pool balance tracking.
3. **Webhook Interface**
   - Expose a simple HTTP endpoint that **lafiya‑web** can call (or use Supabase realtime listeners) to receive push notifications when a profile changes.
   - The webhook payload contains the profile ID and the updated verification state.

## Failure & Replay Handling

- **Persistence**: The cursor is stored in Supabase; if the indexer crashes, it restarts from the last persisted cursor.
- **Idempotency**: Event processing is idempotent – the service checks if an event with the same ledger sequence & index already exists before applying changes.
- **Replay**: In case of extended downtime, the service can perform a **catch‑up sweep** by querying `getEvents` from the last stored cursor up to the latest ledger, processing any missing events.
- **Alerting**: Monitoring (via Supabase functions or external Prometheus) alerts on:
  - Stream disconnects lasting > 1 minute.
  - Polling errors or RPC timeouts.
  - Event processing failures (e.g., DB write errors).

## Implementation

The reference implementation lives in this repository as
[`crates/lafiya-indexer`](../../crates/lafiya-indexer). It uses the polling
approach above, with a Postgres store rather than Supabase, and adds:

- cursor checkpointing that is transactional with each applied page;
- a hard failure (never a silent skip) when the checkpoint falls outside
  RPC retention, plus a `backfill` command for archived history;
- a reconciler that samples `get_attester_status` / `get_attestation`
  and reports mismatches as metrics;
- a read-only, paginated HTTP API with an OpenAPI spec, a health endpoint,
  and Prometheus metrics.

Supabase profile updates and webhooks, described above, can consume the
indexer's API or tables. See the [deployment guide](../indexer.md).

## History Archive Backfill

Soroban RPC `getEvents` only retains roughly the last 7 days of events. A cold
start after longer downtime, or a brand-new deployment, therefore cannot
rebuild state from RPC alone. The indexer models every event producer behind an
`EventSource` trait (`crates/lafiya-indexer/src/rpc.rs`), with two
implementations:

- `RpcEventSource` — live `getEvents` polling against a Soroban RPC node.
- `ArchiveEventSource` — reads a history archive or ledger metadata export
  (per-ledger event batches plus ledger headers) from local disk or object
  storage.

### Checkpoint-hash verification

Archive data is only trusted when it can be tied to the network's history.
Each archive batch carries the ledger header, and the source verifies:

1. the header's hash matches the ledger sequence it claims;
2. the header's `previous_ledger_hash` chains to the preceding verified header;
3. the header hash is anchored in a published checkpoint (or a quorum of
   archive mirrors agree on it).

If any check fails, or the archive does not provide headers at all, the source
is refused with an error and no events from it are applied. Unverifiable
archives are never used to advance the checkpoint.

### Verified archive-to-live handoff

Backfill proceeds from the deployment's genesis ledger up to a **handoff
boundary**: the highest ledger that is both present in the archive and still
within RPC retention. At the boundary the indexer:

1. verifies the archive header for the boundary ledger;
2. fetches the same ledger range from RPC and confirms the event sets match;
3. only then switches to `RpcEventSource` for subsequent ledgers.

If the comparison fails, the handoff is aborted and the checkpoint is not
advanced past the last verified ledger. Backfill is resumable (the checkpoint
records the last applied ledger) and parallelizable by disjoint ledger ranges,
with a final sequential pass to stitch ranges together in order.

The acceptance test for this path is a golden comparison: a fresh database
backfilled from genesis must equal one built purely from live ingestion.

## Single-Writer Coordination

Checkpoints are keyed by name and upserted, so multiple replicas would fight
over them. High-availability deployments therefore elect a single active
ingester:

- **Leadership** is held via a Postgres advisory lock (or a lease table with a
  TTL) acquired by the ingester at startup and renewed while it runs.
- **Fencing tokens**: every checkpoint update carries the leader's monotonically
  increasing fencing token; the store rejects updates from a token older than
  the last accepted one, so a paused or partitioned former leader cannot
  overwrite newer state.
- **Followers** do not ingest; they serve reads from the same database and
  report themselves as followers.
- **Health endpoint** distinguishes `leader` from `follower` and reports
  ingestion lag (latest ledger minus checkpointed ledger) so operators can
  observe failover.

On leader failure the advisory lock is released (or the lease expires) and a
follower acquires leadership with a higher fencing token, resuming from the
last committed checkpoint. Because each page is applied transactionally with
its checkpoint, a leader killed mid-page leaves no partial effect and the new
leader re-processes that page exactly once. Failover time is bounded by the
lease TTL and is measured and documented in the deployment guide, which also
includes a Kubernetes example (leader-elected Deployment plus read replicas).
