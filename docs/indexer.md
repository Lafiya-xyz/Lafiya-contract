# Event Indexer: Deployment Guide

`crates/lafiya-indexer` is the reference indexer for the Lafiya contracts. It
keeps a durable, queryable copy of contract history that contract storage
does not hold and Soroban RPC only retains for a short window (about 7 days
by default). The design rationale is in
[`architecture/event-indexing.md`](architecture/event-indexing.md); event
schemas are in [`events.md`](events.md).

## What it does

| Component | Behaviour |
|---|---|
| **Ingestion** | Polls `getEvents` for the two registry contract IDs. Each page is applied in one Postgres transaction together with the new cursor, so a crash either keeps or discards the whole page, and events already stored are never applied twice. |
| **Retention guard** | Before every page, compares the resume ledger with the RPC's `oldestLedger` (`getHealth`). If history has been pruned past the checkpoint, the process **exits non-zero** with a `retention gap` error naming the missing ledger range. It never skips ahead. |
| **Backfill** | `lafiya-indexer backfill` loads archived events for ranges RPC no longer serves (see below). |
| **Storage** | `raw_events` (append-only audit copy with ledger and tx hash; UPDATE/DELETE are rejected by a trigger), `attesters`, `attestations`, `revocations`, `admin_events`, `checkpoints`. Migrations run automatically on start (sqlx). |
| **Reconciler** | Every `LAFIYA_RECONCILE_INTERVAL_SECS`, samples attesters and records and compares derived state with `get_attester_status` / `get_attestation` (via `simulateTransaction`). Mismatches are logged and counted in metrics. It also fills in region and license hash after `AttesterInfoUpdated`, which carries no data. |
| **API** | Read-only HTTP/JSON, keyset-paginated. Spec: `crates/lafiya-indexer/openapi.yaml`, also served at `/openapi.yaml`. |

### Attester lifecycle

The attester registry stores allowlist membership and suspension
independently, and the indexer mirrors that:

| Event | `allowlisted` | `suspended` | Region / license |
|---|---|---|---|
| `attester_added` | true | unchanged | cleared |
| `attester_suspended` | unchanged | true | unchanged |
| `attester_reinstated` | unchanged | false | unchanged |
| `attester_removed` | false | false | unchanged |
| `attester_info_updated` | unchanged | unchanged | re-read from chain |

`status` is `removed` when not allowlisted, else `suspended` when suspended,
else `active`. Re-adding a suspended attester therefore leaves it
`suspended`, matching the contract.

### Attestations

Each `attestation_recorded` is one row. The record's current attestation is
the latest one not revoked; `attestation_revoked` marks all earlier rows
revoked, because `revoke_attestation` deletes the record's whole on-chain
history.

## API

| Endpoint | Query parameters |
|---|---|
| `GET /attesters` | `region`, `status` (`active` / `suspended` / `removed`), `limit`, `cursor` |
| `GET /attestations` | `attester`, `record_hash`, `limit`, `cursor` |
| `GET /records/{hash}/history` | `limit`, `cursor` |
| `GET /health` | Returns 200 with checkpoint and lag, 503 if Postgres is unreachable. |
| `GET /metrics` | Prometheus text format. |
| `GET /openapi.yaml` | OpenAPI 3 spec. |

List responses are `{ "items": [...], "next_cursor": "..." | null }`; pass
`next_cursor` as `cursor` to get the next page. `limit` defaults to 50 (max 500).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. |
| `LAFIYA_RPC_URL` | required | Soroban RPC endpoint. |
| `LAFIYA_ATTESTER_REGISTRY` | required | `C...` contract ID. |
| `LAFIYA_ATTESTATION_REGISTRY` | required | `C...` contract ID. |
| `LAFIYA_START_LEDGER` | none | First ledger to ingest on an empty database: the ledger the contracts were deployed in. Ignored once a checkpoint exists. |
| `LAFIYA_LISTEN` | `0.0.0.0:8080` | API listen address. |
| `LAFIYA_POLL_INTERVAL_SECS` | `5` | Delay between polls once caught up. |
| `LAFIYA_PAGE_LIMIT` | `200` | `getEvents` page size. |
| `LAFIYA_RECONCILE_INTERVAL_SECS` | `300` | Reconciliation period. |
| `LAFIYA_RECONCILE_SAMPLE` | `50` | Attesters and records sampled per cycle. |
| `LAFIYA_RECONCILE_MAX_LAG` | `2` | Skip the sample while ingestion lags more ledgers than this, so in-flight changes are not reported as mismatches. |
| `RUST_LOG` | `info` | Log filter. |

## Running

```sh
docker build -f crates/lafiya-indexer/Dockerfile -t lafiya-indexer .

docker run -d --name lafiya-indexer -p 8080:8080 \
  -e DATABASE_URL=postgres://indexer:...@db:5432/lafiya \
  -e LAFIYA_RPC_URL=https://soroban-testnet.stellar.org \
  -e LAFIYA_ATTESTER_REGISTRY=C... \
  -e LAFIYA_ATTESTATION_REGISTRY=C... \
  -e LAFIYA_START_LEDGER=<deployment ledger> \
  lafiya-indexer
```

Run one replica. Two replicas would not corrupt state, because pages and
checkpoints are transactional and idempotent, but they duplicate RPC load.
Use a restart policy (`--restart on-failure` or a Kubernetes Deployment), but
**alert on a retention-gap exit rather than letting it crash-loop silently**:
restarts cannot fix it, only a backfill can.

### Operations checklist

- Keep downtime well under the RPC retention window. The indexer resumes
  from its checkpoint after any restart shorter than that.
- Use a Postgres backup policy that matches your audit needs. `raw_events`
  is the authoritative audit copy.
- Point `/health` at your liveness probe.

### Metrics and alerts

| Metric | Suggested alert |
|---|---|
| `lafiya_indexer_lag_ledgers` | > 60 for 5 minutes (about 5 minutes behind). |
| `rate(lafiya_indexer_events_total[1m])` | Events per second, for dashboards. |
| `lafiya_indexer_ingest_errors_total` | Increasing steadily (RPC or database problems; retried with backoff). |
| `lafiya_indexer_reconcile_mismatches_total{kind}` | Any sustained increase: derived state disagrees with chain. |
| `lafiya_indexer_checkpoint_ledger` | Not advancing. |
| process exit with `retention gap` | Page immediately: backfill needed. |

## Backfill from archived history

**Chosen source:** the
[Stellar Hubble](https://developers.stellar.org/docs/data/analytics/hubble)
public BigQuery dataset. It is derived from full ledger history (via Galexie
and stellar-etl) and exposes contract events with their XDR topics and data.
Any archive works if it can be exported in the format below: a Galexie data
lake processed with `stellar-etl`, or a captive-core history source.

The `backfill` command reads **newline-delimited JSON** where each line is a
`getEvents` event object:

```json
{"id":"0000000429496733696-0000000000","ledger":100,"ledgerClosedAt":"2026-01-01T00:00:00Z","contractId":"C...","txHash":"...","topic":["<base64 ScVal>", "..."],"value":"<base64 ScVal>","inSuccessfulContractCall":true}
```

`id` must be the RPC event id (`<19-digit TOID>-<10-digit event index>`),
which sorts in chain order. When exporting from Hubble, filter on the two
contract IDs and on successful contract events, then map the table's ledger,
transaction and operation columns into this shape. Check the column names
against the current Hubble schema.

Procedure:

1. Export all events for the two contracts from the deployment ledger up to
   a ledger `N` that RPC still retains (`oldestLedger` from `getHealth`).
2. On an empty database (or one whose checkpoint is before the export's
   first ledger), run:

   ```sh
   lafiya-indexer backfill --file events.ndjson --through-ledger N
   ```

   `--through-ledger` declares the export complete through ledger `N`, so
   live ingestion resumes at `N + 1`. Without it, ingestion resumes right
   after the last exported event.
3. Start `lafiya-indexer run`. If `N + 1` has already been pruned by RPC, it
   exits with a retention-gap error: export a later `N` and repeat.

The backfill refuses to run behind an existing checkpoint, because derived
state must be built in chain order. To rebuild from scratch, drop the
database and backfill again.

## Tests

```sh
# Unit tests (no database needed)
cargo test -p lafiya-indexer

# Database tests: restart/resume, retention-gap, backfill, API
DATABASE_URL=postgres://postgres:pg@localhost:5432/postgres cargo test -p lafiya-indexer

# End to end against a local quickstart (deploys contracts, drives a scenario,
# indexes it, and asserts the API equals contract state)
docker run -d -p 8000:8000 stellar/quickstart:testing --local --enable rpc
DATABASE_URL=postgres://postgres:pg@localhost:5432/postgres crates/lafiya-indexer/tests/e2e.sh
```

Database tests create an isolated schema per test and are skipped when
`DATABASE_URL` is unset.
