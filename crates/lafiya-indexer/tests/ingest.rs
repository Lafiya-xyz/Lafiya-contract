//! Ingestion against a mocked event source and a real Postgres
//! (`DATABASE_URL`; skipped when unset).

mod common;

use std::sync::{Arc, Mutex};

use lafiya_indexer::decode::RawEvent;
use lafiya_indexer::ingest::{backfill, IngestError, Ingestor};
use lafiya_indexer::metrics::Metrics;
use lafiya_indexer::rpc::{EventPage, EventSource, LedgerWindow, RpcError, Start};
use lafiya_indexer::store;
use sqlx::PgPool;
use stellar_xdr::{
    AccountId, Limits, PublicKey, ScAddress, ScMap, ScMapEntry, ScSymbol, ScVal, Uint256, WriteXdr,
};

#[derive(Clone, Default)]
struct MockSource {
    events: Arc<Mutex<Vec<RawEvent>>>,
    window: Arc<Mutex<(u32, u32)>>,
}

impl MockSource {
    fn new(events: Vec<RawEvent>, oldest: u32, latest: u32) -> Self {
        Self {
            events: Arc::new(Mutex::new(events)),
            window: Arc::new(Mutex::new((oldest, latest))),
        }
    }
}

fn event_id(ledger: u32, index: u32) -> String {
    format!("{:019}-{index:010}", (u64::from(ledger) << 32) | (1 << 12))
}

impl EventSource for MockSource {
    async fn ledger_window(&self) -> Result<LedgerWindow, RpcError> {
        let (oldest_ledger, latest_ledger) = *self.window.lock().unwrap();
        Ok(LedgerWindow {
            oldest_ledger,
            latest_ledger,
        })
    }

    async fn get_events(&self, start: &Start, limit: u32) -> Result<EventPage, RpcError> {
        let (oldest, latest) = *self.window.lock().unwrap();
        let after = match start {
            Start::Ledger(l) if *l < oldest => {
                return Err(RpcError::Rpc {
                    code: -32600,
                    message: format!("startLedger must be between the oldest ledger: {oldest}"),
                })
            }
            Start::Ledger(l) => format!("{:019}", u64::from(*l) << 32),
            Start::Cursor(c) => c.clone(),
        };
        let events: Vec<RawEvent> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.id > after)
            .take(limit as usize)
            .cloned()
            .collect();
        let cursor = events.last().map(|e| e.id.clone()).unwrap_or(after);
        Ok(EventPage {
            events,
            cursor,
            latest_ledger: latest,
        })
    }
}

fn b64(v: ScVal) -> String {
    v.to_xdr_base64(Limits::none()).unwrap()
}

fn sym(s: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(s.try_into().unwrap()))
}

fn account(n: u8) -> (ScVal, String) {
    let a = ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([n; 32]))));
    let s = a.to_string();
    (ScVal::Address(a), s)
}

fn ev(ledger: u32, index: u32, topic: Vec<ScVal>, value: ScVal) -> RawEvent {
    RawEvent {
        id: event_id(ledger, index),
        ledger,
        ledger_closed_at: "2026-09-01T00:00:00Z".into(),
        contract_id: "CCONTRACT".into(),
        tx_hash: format!("{ledger:064}"),
        topic: topic.into_iter().map(b64).collect(),
        value: b64(value),
        in_successful_contract_call: true,
    }
}

fn attester_event(ledger: u32, name: &str, who: u8) -> RawEvent {
    ev(ledger, 0, vec![sym(name), account(who).0], ScVal::Map(None))
}

fn attested(ledger: u32, hash: u8, who: u8, timestamp: u64) -> RawEvent {
    let data = ScVal::Map(Some(ScMap(
        vec![
            ScMapEntry {
                key: sym("attester"),
                val: account(who).0,
            },
            ScMapEntry {
                key: sym("timestamp"),
                val: ScVal::U64(timestamp),
            },
        ]
        .try_into()
        .unwrap(),
    )));
    let hash = ScVal::Bytes(vec![hash; 32].try_into().unwrap());
    ev(ledger, 0, vec![sym("attestation_recorded"), hash], data)
}

fn revoked(ledger: u32, hash: u8) -> RawEvent {
    let hash = ScVal::Bytes(vec![hash; 32].try_into().unwrap());
    ev(
        ledger,
        0,
        vec![sym("attestation_revoked"), hash],
        ScVal::Map(None),
    )
}

/// add A, add B, suspend B, attest R1 (A), revoke R1, attest R2 (A),
/// remove A, re-add B (stays suspended).
fn scenario() -> Vec<RawEvent> {
    vec![
        attester_event(100, "attester_added", 1),
        attester_event(101, "attester_added", 2),
        attester_event(102, "attester_suspended", 2),
        attested(103, 0xaa, 1, 1_000),
        revoked(104, 0xaa),
        attested(105, 0xbb, 1, 2_000),
        attester_event(106, "attester_removed", 1),
        attester_event(107, "attester_added", 2),
    ]
}

fn ingestor(pool: &PgPool, source: &MockSource) -> Ingestor<MockSource> {
    Ingestor {
        pool: pool.clone(),
        source: source.clone(),
        metrics: Arc::new(Metrics::new()),
        start_ledger: Some(100),
        page_limit: 3,
    }
}

async fn statuses(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as("SELECT address, status FROM attesters ORDER BY address")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn assert_final_state(pool: &PgPool) {
    let mut expected = vec![
        (account(1).1, "removed".to_string()),
        (account(2).1, "suspended".to_string()),
    ];
    expected.sort();
    assert_eq!(statuses(pool).await, expected);

    let raw: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_events")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(raw, 8);
    let attestations: Vec<(String, bool)> = sqlx::query_as(
        "SELECT record_hash, revoked_event_id IS NOT NULL FROM attestations ORDER BY event_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        attestations,
        vec![("aa".repeat(32), true), ("bb".repeat(32), false)]
    );
}

#[tokio::test]
async fn resumes_after_restart_without_loss_or_duplicates() {
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let source = MockSource::new(scenario(), 1, 110);

    // First run ingests one page, then "crashes".
    let step = ingestor(&pool, &source).step().await.unwrap();
    assert_eq!(step.applied, 3);
    assert!(!step.caught_up);
    let cp = store::checkpoint(&pool).await.unwrap().unwrap();
    assert_eq!(cp.cursor.as_deref(), Some(event_id(102, 0).as_str()));

    // A new process resumes from the persisted cursor.
    let restarted = ingestor(&pool, &source);
    loop {
        if restarted.step().await.unwrap().caught_up {
            break;
        }
    }
    assert_final_state(&pool).await;

    // Re-applying already-ingested events changes nothing.
    let applied = store::apply_page(&pool, &scenario(), &cp).await.unwrap();
    assert_eq!(applied, 0);
    assert_final_state(&pool).await;
}

#[tokio::test]
async fn fails_loudly_when_cursor_falls_outside_retention() {
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let source = MockSource::new(scenario()[..2].to_vec(), 1, 110);
    let ingestor = ingestor(&pool, &source);
    while !ingestor.step().await.unwrap().caught_up {}
    let before = store::checkpoint(&pool).await.unwrap();

    // The indexer was down while RPC pruned past its cursor.
    *source.window.lock().unwrap() = (500, 900);
    let err = ingestor.step().await.unwrap_err();
    assert!(
        matches!(
            err,
            IngestError::RetentionGap {
                resume: 101,
                oldest: 500
            }
        ),
        "{err}"
    );
    assert!(err.is_fatal());
    // The run loop stops instead of skipping ahead.
    let err = ingestor.run(std::time::Duration::from_millis(1)).await;
    assert!(matches!(err, IngestError::RetentionGap { .. }));
    assert_eq!(store::checkpoint(&pool).await.unwrap(), before);
}

#[tokio::test]
async fn fails_loudly_when_start_ledger_is_already_pruned() {
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let source = MockSource::new(scenario(), 500, 900);
    let err = ingestor(&pool, &source).step().await.unwrap_err();
    assert!(matches!(
        err,
        IngestError::RetentionGap {
            resume: 100,
            oldest: 500
        }
    ));
    assert_eq!(store::checkpoint(&pool).await.unwrap(), None);
}

#[tokio::test]
async fn backfill_then_live_ingestion() {
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let all = scenario();
    // Ledgers 100..=103 only exist in the archive; RPC retains 104 onwards.
    let archived = all[..4].to_vec();
    assert_eq!(
        backfill(&pool, archived.clone(), Some(103)).await.unwrap(),
        4
    );
    // Backfilling the same range again would apply events out of order.
    assert!(backfill(&pool, archived, Some(103)).await.is_err());

    let live: Vec<RawEvent> = all[4..].to_vec();
    let source = MockSource::new(live, 104, 110);
    let ingestor = ingestor(&pool, &source);
    while !ingestor.step().await.unwrap().caught_up {}
    assert_final_state(&pool).await;
}

#[tokio::test]
async fn api_serves_derived_state() {
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let source = MockSource::new(scenario(), 1, 110);
    let ingestor = ingestor(&pool, &source);
    while !ingestor.step().await.unwrap().caught_up {}

    let app = lafiya_indexer::api::router(lafiya_indexer::api::AppState {
        pool,
        metrics: ingestor.metrics.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let get = |path: &str| {
        let url = format!("{base}{path}");
        async move {
            let resp = reqwest::get(url).await.unwrap();
            (
                resp.status().as_u16(),
                resp.json::<serde_json::Value>().await.unwrap(),
            )
        }
    };

    let (status, body) = get("/attesters?status=suspended").await;
    assert_eq!(status, 200);
    assert_eq!(body["items"][0]["address"], account(2).1);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let (_, page1) = get("/attesters?limit=1").await;
    let cursor = page1["next_cursor"].as_str().unwrap().to_string();
    let (_, page2) = get(&format!("/attesters?limit=1&cursor={cursor}")).await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert!(page2["next_cursor"].is_null());

    let (_, body) = get(&format!("/attestations?attester={}", account(1).1)).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let (_, body) = get(&format!("/records/{}/history", "aa".repeat(32))).await;
    let kinds: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["attested", "revoked"]);

    assert_eq!(get("/records/nothex/history").await.0, 400);
    assert_eq!(get("/attesters?status=bogus").await.0, 400);
    assert_eq!(get("/health").await.1["status"], "ok");
}
