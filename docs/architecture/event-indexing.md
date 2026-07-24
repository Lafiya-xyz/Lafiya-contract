# Event Indexing Design Spec

## Overview

Lafiya contracts currently declare the following on-chain event schemas:

- `AdminTransferred` (`attester-registry` and `attestation-registry`)
- `Initialized`
- `AttesterAdded`
- `AttesterRemoved`
- `AttesterSuspended`
- `AttesterReinstated`
- `AttestationRecorded`
- `AttestationRevoked`
- `Upgraded` (`attester-registry`)

`Initialized` is currently a declared schema only: neither registry publishes it
during initialization. Indexers must not rely on receiving it unless contract
behavior is changed in a future release.

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
   - For `AttesterAdded` / `AttesterRemoved`, add or remove the account in a secondary `attesters` table.
   - For `AttesterSuspended` / `AttesterReinstated`, update the account's active status without losing its allowlist history.
   - Record `AdminTransferred` as a contract-administration audit event; it does not directly change profile verification state.
   - If a future contract release begins publishing `Initialized`, record it as an administration audit event as well.
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

## Repository Ownership

- The indexer code should live in a **dedicated repository** (e.g., `lafiya-event-indexer`). This keeps the on‑chain contracts repository focused on smart‑contract logic.
- A short‑term plan is to create the repository under the organization and add a `README.md` linking to this design doc.
- Follow‑up implementation tickets will be created in that repo (e.g., `#1 Implement streaming client`, `#2 Supabase schema migration`).

## Next Steps

1. **Create repository** `lafiya-event-indexer` (or decide to host within `lafiya‑web` if maintainers prefer).
2. Add the `event-indexing.md` design doc (this file) to the repo's `docs/architecture` folder.
3. Draft implementation tickets as described above.
4. Review the design with the maintainer of `lafiya‑web` and update according to feedback.

---
*This design spec is intended for review only; no code changes are made in this repository.*
