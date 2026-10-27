//! Prometheus metrics for the indexer.
//!
//! # Metric catalogue (stable names)
//!
//! All metric names below are part of the public observability contract and are
//! considered **stable**: they may be added to, but existing names, types, and
//! label sets will not change without a deprecation cycle. Dashboards, recording
//! rules, and alert definitions in `docs/observability.md` and
//! `docs/runbooks/` depend on them.
//!
//! | Metric | Type | Labels | Meaning |
//! | --- | --- | --- | --- |
//! | `lafiya_indexer_lag_ledgers` | gauge | – | Ledgers between RPC latest and checkpoint |
//! | `lafiya_indexer_lag_seconds` | gauge | – | Estimated seconds of ingestion lag |
//! | `lafiya_indexer_checkpoint_ledger` | gauge | – | Ledger of the ingestion checkpoint |
//! | `lafiya_indexer_checkpoint_age_seconds` | gauge | – | Wall-clock age of the checkpoint |
//! | `lafiya_indexer_events_total` | counter | – | Contract events ingested |
//! | `lafiya_indexer_ingest_errors_total` | counter | – | Failed ingestion steps (retried) |
//! | `lafiya_indexer_reconcile_checks_total` | counter | `kind` | Entities compared against chain |
//! | `lafiya_indexer_reconcile_mismatches_total` | counter | `kind` | Derived state differing from chain |
//! | `lafiya_indexer_api_requests_total` | counter | `route`, `status` | API requests served |
//! | `lafiya_indexer_api_request_duration_seconds` | histogram | `route` | API latency (p99 SLI) |
//!
//! # Cardinality / privacy policy
//!
//! Labels MUST be drawn from small, bounded sets. Never label a metric with a
//! record hash, account address, transaction hash, or any other unbounded or
//! linkable value (see `docs/compliance/`). `route` is the templated route
//! pattern (e.g. `/attestations/:id`), not the concrete path; `status` is the
//! HTTP status class/code. Events per second is
//! `rate(lafiya_indexer_events_total[1m])`.

use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};

/// Upper bounds (seconds) for the API latency histogram. Bounded buckets keep
/// the series count fixed regardless of traffic.
const API_LATENCY_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

pub struct Metrics {
    registry: Registry,
    /// Ledgers between the RPC's latest ledger and the ingestion checkpoint.
    pub lag_ledgers: IntGauge,
    /// Estimated seconds of ingestion lag (ledgers behind × ledger time).
    pub lag_seconds: IntGauge,
    pub checkpoint_ledger: IntGauge,
    /// Wall-clock age in seconds of the ingestion checkpoint.
    pub checkpoint_age_seconds: IntGauge,
    pub events_total: IntCounter,
    pub ingest_errors_total: IntCounter,
    /// Labelled by `kind` (`attester` / `attestation`).
    pub reconcile_checks_total: IntCounterVec,
    pub reconcile_mismatches_total: IntCounterVec,
    /// Ledger-hash continuity mismatches detected during ingestion.
    pub reorg_detected_total: IntCounter,
    /// Depth (in ledgers) of the most recent rollback; 0 when none has occurred.
    pub reorg_last_rollback_depth: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let gauge = |name: &str, help: &str| {
            let g = IntGauge::new(name, help).expect("valid metric");
            registry
                .register(Box::new(g.clone()))
                .expect("unique metric");
            g
        };
        let counter = |name: &str, help: &str| {
            let c = IntCounter::new(name, help).expect("valid metric");
            registry
                .register(Box::new(c.clone()))
                .expect("unique metric");
            c
        };
        let counter_vec = |name: &str, help: &str, labels: &[&str]| {
            let c = IntCounterVec::new(Opts::new(name, help), labels).expect("valid metric");
            registry
                .register(Box::new(c.clone()))
                .expect("unique metric");
            c
        };
        let histogram = |name: &str, help: &str, labels: &[&str]| {
            let h = Histogram::with_opts(
                HistogramOpts::new(name, help).buckets(API_LATENCY_BUCKETS.to_vec()),
            )
            .expect("valid metric");
            registry
                .register(Box::new(h.clone()))
                .expect("unique metric");
            h
        };
        Self {
            lag_ledgers: gauge(
                "lafiya_indexer_lag_ledgers",
                "Ledgers between the RPC latest ledger and the ingestion checkpoint",
            ),
            lag_seconds: gauge(
                "lafiya_indexer_lag_seconds",
                "Estimated seconds of ingestion lag",
            ),
            checkpoint_ledger: gauge(
                "lafiya_indexer_checkpoint_ledger",
                "Ledger of the ingestion checkpoint",
            ),
            checkpoint_age_seconds: gauge(
                "lafiya_indexer_checkpoint_age_seconds",
                "Wall-clock age in seconds of the ingestion checkpoint",
            ),
            events_total: counter("lafiya_indexer_events_total", "Contract events ingested"),
            ingest_errors_total: counter(
                "lafiya_indexer_ingest_errors_total",
                "Failed ingestion steps (retried)",
            ),
            reconcile_checks_total: counter_vec(
                "lafiya_indexer_reconcile_checks_total",
                "Entities compared against on-chain storage",
                &["kind"],
            ),
            reconcile_mismatches_total: counter_vec(
                "lafiya_indexer_reconcile_mismatches_total",
                "Entities whose derived state differs from on-chain storage",
                &["kind"],
            ),
            api_requests_total: counter_vec(
                "lafiya_indexer_api_requests_total",
                "API requests served",
                &["route", "status"],
            ),
            api_request_duration_seconds: histogram(
                "lafiya_indexer_api_request_duration_seconds",
                "API request latency in seconds",
                &["route"],
            ),
            reorg_detected_total: counter(
                "lafiya_indexer_reorg_detected_total",
                "Ledger-hash continuity mismatches detected during ingestion",
            ),
            reorg_last_rollback_depth: gauge(
                "lafiya_indexer_reorg_last_rollback_depth",
                "Depth in ledgers of the most recent rollback (0 if none)",
            ),
            rate_limit_denials_total: counter_vec2(
                "lafiya_indexer_rate_limit_denials_total",
                "API requests denied by rate limits or budgets",
                &["scope", "tier"],
            ),
            rate_limit_allowed_total: counter_vec2(
                "lafiya_indexer_rate_limit_allowed_total",
                "API requests admitted by rate limits",
                &["scope", "tier"],
            ),
            is_leader: gauge(
                "lafiya_indexer_is_leader",
                "1 when this replica is the active ingester, 0 for read-only followers",
            ),
            follower_lag_ledgers: gauge(
                "lafiya_indexer_follower_lag_ledgers",
                "Ledgers between the leader checkpoint and this replica's view",
            ),
            stale_writer_rejections_total: counter(
                "lafiya_indexer_stale_writer_rejections_total",
                "Checkpoint writes rejected because the fencing token was stale",
            ),
            registry,
        }
    }

    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buf)
            .expect("text encoding cannot fail");
        String::from_utf8(buf).expect("prometheus text is utf-8")
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
