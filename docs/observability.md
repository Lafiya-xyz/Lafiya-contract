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
