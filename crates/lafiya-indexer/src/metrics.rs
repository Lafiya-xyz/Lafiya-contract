//! Prometheus metrics. Events per second is `rate(lafiya_indexer_events_total[1m])`.

use prometheus::{Encoder, IntCounter, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};

pub struct Metrics {
    registry: Registry,
    /// Ledgers between the RPC's latest ledger and the ingestion checkpoint.
    pub lag_ledgers: IntGauge,
    pub checkpoint_ledger: IntGauge,
    pub events_total: IntCounter,
    pub ingest_errors_total: IntCounter,
    /// Labelled by `kind` (`attester` / `attestation`).
    pub reconcile_checks_total: IntCounterVec,
    pub reconcile_mismatches_total: IntCounterVec,
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
        let counter_vec = |name: &str, help: &str| {
            let c = IntCounterVec::new(Opts::new(name, help), &["kind"]).expect("valid metric");
            registry
                .register(Box::new(c.clone()))
                .expect("unique metric");
            c
        };
        Self {
            lag_ledgers: gauge(
                "lafiya_indexer_lag_ledgers",
                "Ledgers between the RPC latest ledger and the ingestion checkpoint",
            ),
            checkpoint_ledger: gauge(
                "lafiya_indexer_checkpoint_ledger",
                "Ledger of the ingestion checkpoint",
            ),
            events_total: counter("lafiya_indexer_events_total", "Contract events ingested"),
            ingest_errors_total: counter(
                "lafiya_indexer_ingest_errors_total",
                "Failed ingestion steps (retried)",
            ),
            reconcile_checks_total: counter_vec(
                "lafiya_indexer_reconcile_checks_total",
                "Entities compared against on-chain storage",
            ),
            reconcile_mismatches_total: counter_vec(
                "lafiya_indexer_reconcile_mismatches_total",
                "Entities whose derived state differs from on-chain storage",
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
