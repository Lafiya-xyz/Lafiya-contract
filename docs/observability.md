# Observability: logs, traces, and the operation audit trail

## Structured logs

`lafiya-cli` logs through `tracing` to **stderr**, so command output on stdout
(`config env`, `auth decode --format json`) stays pipeable.

| Flag / env | Effect |
| --- | --- |
| `--log-format text` (default) | Human-readable lines |
| `--log-format json` | One JSON object per event, with the current span and span list |
| `-v` / `-vv` | `debug` / `trace` |
| `LAFIYA_LOG=<filter>` | Full `EnvFilter` syntax, overrides `-v` |

Mutating commands run inside an `operation` span (`command`, `network`,
`contract`, `function`) and log each stellar CLI call with latency and outcome.
`lafiya-rpc-resilience` emits an event per RPC call (`rpc submit`,
`rpc get_transaction`) with provider, latency, and classified outcome.

### Redaction

Every byte a log layer writes passes through `logging::redact`, which replaces
Stellar secret seeds (`S...`, 56 chars) and mnemonic phrases (12+ lowercase
words) with `[REDACTED]`. The stellar CLI's own stderr is passed through the
same filter. Tests: `cargo test -p lafiya-cli logging`.

## Operation audit log

Every mutating command (`attester add`, `attester remove`) appends a record to
`~/.lafiya/audit.jsonl` (override with `LAFIYA_AUDIT_LOG`), whether it succeeded
or failed:

```json
{"seq":0,"timestamp":1790000000,"command":"attester add","network":"testnet","contract":"C...","function":"add_attester","arg_hashes":["<sha256>"],"signer":"admin","tx_hash":"<hash or null>","outcome":"success","prev_hash":"000...","hash":"<sha256>"}
```

- Arguments are stored only as SHA-256 hashes; strings are redacted.
- Each record's `hash` covers all its fields plus `prev_hash`, so editing,
  reordering, or deleting a record breaks the chain.

```bash
lafiya-cli audit show     # print records
lafiya-cli audit verify   # check the hash chain; non-zero exit when broken
```

The chain detects tampering after the fact, not a user rewriting the whole
file; ship the file to append-only storage if that matters.

## OpenTelemetry (OTLP) export

Build with the `otel` feature and set the standard OTLP endpoint variable.
Spans are exported over OTLP/HTTP (protobuf) as service `lafiya-cli`.

Local demo with the OpenTelemetry Collector printing spans to its console:

```bash
cat > otel-collector.yaml <<'YAML'
receivers:
  otlp:
    protocols:
      http:
        endpoint: 0.0.0.0:4318
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [debug]
YAML
docker run --rm -p 4318:4318 -v "$PWD/otel-collector.yaml:/etc/otelcol/config.yaml" \
  otel/opentelemetry-collector:latest

# In another shell:
cargo build -p lafiya-cli --features otel
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
  ./target/debug/lafiya-cli --network testnet auth decode "$(cat entry.xdr)"
```

The collector prints an `auth.decode` span with `service.name: lafiya-cli`.
Point the endpoint at Jaeger, Tempo, or Honeycomb in the same way. Without
`OTEL_EXPORTER_OTLP_ENDPOINT` the exporter stays off even when compiled in.

## Indexer SLOs, recording rules, and alerts

`lafiya-indexer` exposes Prometheus metrics on `/metrics` (see
`crates/lafiya-indexer/src/api.rs`). The metric names below are **stable**:
renaming one is a breaking change and requires updating the rules, the
dashboard, and this document together.

### Stable metric names

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `lafiya_indexer_ingest_lag_ledgers` | gauge | — | Ledgers between the chain tip and the last ingested ledger |
| `lafiya_indexer_ingest_lag_seconds` | gauge | — | Wall-clock age of the last ingested ledger |
| `lafiya_indexer_checkpoint_age_seconds` | gauge | — | Age of the last persisted checkpoint |
| `lafiya_indexer_http_requests_total` | counter | `method`, `route`, `status` | API requests, by status class |
| `lafiya_indexer_http_request_duration_seconds` | histogram | `method`, `route` | API request latency |
| `lafiya_indexer_up` | gauge | — | `1` while the indexer process is serving |

### SLIs and SLO targets

| SLI | Definition | SLO target |
| --- | --- | --- |
| Ingestion lag (ledgers) | `lafiya_indexer_ingest_lag_ledgers` | ≤ 5 ledgers for 99% of a 30d window |
| Ingestion lag (seconds) | `lafiya_indexer_ingest_lag_seconds` | ≤ 30s for 99% of a 30d window |
| API error ratio | `5xx / total` from `lafiya_indexer_http_requests_total` | < 1% over 5m and 1h |
| API p99 latency | `histogram_quantile(0.99, ...)` over `lafiya_indexer_http_request_duration_seconds` | ≤ 500ms over 5m |
| Checkpoint age | `lafiya_indexer_checkpoint_age_seconds` | ≤ 120s |

### Recording rules

Committed in [`deploy/prometheus/lafiya-indexer.rules.yaml`](../deploy/prometheus/lafiya-indexer.rules.yaml):

| Recorded series | Expression summary |
| --- | --- |
| `lafiya:indexer_ingest_lag_ledgers:max` | max ingest lag in ledgers |
| `lafiya:indexer_ingest_lag_seconds:max` | max ingest lag in seconds |
| `lafiya:indexer_api_error_ratio:rate5m` | 5xx ratio over 5m |
| `lafiya:indexer_api_error_ratio:rate1h` | 5xx ratio over 1h |
| `lafiya:indexer_api_latency_p99:5m` | p99 latency over 5m |
| `lafiya:indexer_checkpoint_age_seconds:max` | max checkpoint age |

### Alerts

Each alert carries a `runbook_url` annotation pointing at a section of
[`docs/runbooks/indexer.md`](runbooks/indexer.md).

| Alert | Condition | Runbook section |
| --- | --- | --- |
| `LafiyaIndexerDown` | `lafiya_indexer_up == 0` for 2m | [Indexer down](runbooks/indexer.md#indexer-down) |
| `LafiyaIndexerIngestLagLedgers` | lag > 5 ledgers for 10m | [Ingestion lag](runbooks/indexer.md#ingestion-lag) |
| `LafiyaIndexerIngestLagSeconds` | lag > 30s for 10m | [Ingestion lag](runbooks/indexer.md#ingestion-lag) |
| `LafiyaIndexerAPIErrorRatio` | 5xx ratio > 1% for 10m | [API error ratio](runbooks/indexer.md#api-error-ratio) |
| `LafiyaIndexerAPILatencyP99` | p99 > 500ms for 10m | [API latency](runbooks/indexer.md#api-latency) |
| `LafiyaIndexerCheckpointStale` | checkpoint age > 120s for 10m | [Stale checkpoint](runbooks/indexer.md#stale-checkpoint) |

### Dashboard

The Grafana dashboard lives at
[`deploy/grafana/lafiya-indexer.json`](../deploy/grafana/lafiya-indexer.json)
and charts every recorded series above.

### CI validation

`promtool check rules` and `promtool test rules` run in CI against the rule
file and its unit tests (`deploy/prometheus/lafiya-indexer.rules.test.yaml`),
so every alert has at least one firing and one non-firing case. The dashboard
JSON is validated as parseable JSON in the same job.
