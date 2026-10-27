#![allow(dead_code)]

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

/// A pool on a fresh, migrated schema, or `None` (test skipped) when
/// `DATABASE_URL` is not set.
pub async fn test_pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping database test");
        return None;
    };
    let schema = format!("test_{}", rand::random::<u32>());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |conn, _| {
            let schema = schema.clone();
            Box::pin(async move {
                conn.execute(format!("SET search_path TO {schema}").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    lafiya_indexer::store::MIGRATOR.run(&pool).await.unwrap();
    Some(pool)
}

/// A single ledger as served by a (possibly forked) provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerPage {
    pub sequence: i64,
    pub hash: String,
    /// Events contained in this ledger, in ingest order.
    pub events: Vec<String>,
}

impl LedgerPage {
    pub fn new(sequence: i64, hash: impl Into<String>, events: Vec<String>) -> Self {
        Self {
            sequence,
            hash: hash.into(),
            events,
        }
    }
}

/// A deterministic, in-memory provider history used to drive the ingest loop
/// in tests. Two histories can share a common prefix and then diverge, which
/// models a provider failover serving a forked tip.
#[derive(Clone, Debug)]
pub struct FakeProvider {
    pub name: String,
    pub ledgers: Vec<LedgerPage>,
}

impl FakeProvider {
    pub fn new(name: impl Into<String>, ledgers: Vec<LedgerPage>) -> Self {
        Self {
            name: name.into(),
            ledgers,
        }
    }

    /// Build a linear history starting at `start` with `len` ledgers. Each
    /// ledger hash is derived from `(tag, sequence)` so histories with the
    /// same tag share hashes and histories with different tags fork.
    pub fn linear(name: impl Into<String>, tag: &str, start: i64, len: i64) -> Self {
        let ledgers = (start..start + len)
            .map(|seq| {
                let hash = ledger_hash(tag, seq);
                let events = vec![format!("{tag}:{seq}:event")];
                LedgerPage::new(seq, hash, events)
            })
            .collect();
        Self::new(name, ledgers)
    }

    /// Return the ledger at `sequence`, if this provider serves it.
    pub fn ledger(&self, sequence: i64) -> Option<&LedgerPage> {
        self.ledgers.iter().find(|l| l.sequence == sequence)
    }

    /// Return the page of ledgers starting at `cursor` (inclusive), up to
    /// `limit` entries. Mirrors the RPC cursor pagination the ingest loop uses.
    pub fn page(&self, cursor: i64, limit: usize) -> Vec<LedgerPage> {
        self.ledgers
            .iter()
            .filter(|l| l.sequence >= cursor)
            .take(limit)
            .cloned()
            .collect()
    }

    /// The highest ledger sequence this provider serves.
    pub fn tip(&self) -> Option<i64> {
        self.ledgers.iter().map(|l| l.sequence).max()
    }
}

/// Deterministic ledger hash for a given history tag and sequence.
pub fn ledger_hash(tag: &str, sequence: i64) -> String {
    format!("{tag}-{sequence:020}")
}

/// Build two providers that share a common prefix of `common` ledgers starting
/// at `start`, then diverge: `a` continues with tag `tag_a` and `b` continues
/// with tag `tag_b`. Returns `(provider_a, provider_b, fork_sequence)` where
/// `fork_sequence` is the first sequence at which the histories differ.
pub fn forked_providers(
    start: i64,
    common: i64,
    tail: i64,
    tag_a: &str,
    tag_b: &str,
) -> (FakeProvider, FakeProvider, i64) {
    let shared_tag = "shared";
    let mut a = Vec::new();
    let mut b = Vec::new();
    for seq in start..start + common {
        let hash = ledger_hash(shared_tag, seq);
        let events = vec![format!("{shared_tag}:{seq}:event")];
        a.push(LedgerPage::new(seq, hash.clone(), events.clone()));
        b.push(LedgerPage::new(seq, hash, events));
    }
    let fork_sequence = start + common;
    for seq in fork_sequence..fork_sequence + tail {
        a.push(LedgerPage::new(
            seq,
            ledger_hash(tag_a, seq),
            vec![format!("{tag_a}:{seq}:event")],
        ));
        b.push(LedgerPage::new(
            seq,
            ledger_hash(tag_b, seq),
            vec![format!("{tag_b}:{seq}:event")],
        ));
    }
    (
        FakeProvider::new("provider_a", a),
        FakeProvider::new("provider_b", b),
        fork_sequence,
    )
}

/// A provider that can be swapped mid-stream, modelling a failover between
/// RPC endpoints. The ingest loop pulls pages from whichever provider is
/// currently active.
#[derive(Clone, Debug)]
pub struct SwappableProvider {
    active: FakeProvider,
}

impl SwappableProvider {
    pub fn new(active: FakeProvider) -> Self {
        Self { active }
    }

    /// Swap the active provider, returning the previous one.
    pub fn swap(&mut self, next: FakeProvider) -> FakeProvider {
        std::mem::replace(&mut self.active, next)
    }

    pub fn active(&self) -> &FakeProvider {
        &self.active
    }

    pub fn page(&self, cursor: i64, limit: usize) -> Vec<LedgerPage> {
        self.active.page(cursor, limit)
    }
}