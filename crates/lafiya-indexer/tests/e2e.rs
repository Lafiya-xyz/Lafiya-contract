//! End-to-end: index real contracts on a local quickstart and assert the API
//! agrees with on-chain storage. Driven by `tests/e2e.sh`, which deploys the
//! contracts, runs a scenario, and sets:
//!
//!   DATABASE_URL, LAFIYA_E2E_RPC_URL, LAFIYA_E2E_START_LEDGER,
//!   LAFIYA_E2E_ATTESTER_REGISTRY, LAFIYA_E2E_ATTESTATION_REGISTRY,
//!   LAFIYA_E2E_INCENTIVE_POOL
//!
//! Skipped unless those variables are set.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use lafiya_indexer::api::{router, AppState};
use lafiya_indexer::archive::ArchiveSource;
use lafiya_indexer::ingest::Ingestor;
use lafiya_indexer::metrics::Metrics;
use lafiya_indexer::reconcile::{ChainAttestation, Reconciler};
use lafiya_indexer::rpc::RpcClient;
use lafiya_indexer::store::Store;
use serde_json::Value;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

async fn get_all(base: &str, path: &str) -> Vec<Value> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = format!("{base}{path}{sep}limit=2");
        if let Some(c) = &cursor {
            url.push_str(&format!("&cursor={c}"));
        }
        let body: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
        items.extend(body["items"].as_array().unwrap().iter().cloned());
        match body["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => return items,
        }
    }
}

#[tokio::test]
async fn api_matches_contract_state() {
    let (Some(rpc_url), Some(start), Some(attester_registry), Some(attestation_registry)) = (
        env("LAFIYA_E2E_RPC_URL"),
        env("LAFIYA_E2E_START_LEDGER"),
        env("LAFIYA_E2E_ATTESTER_REGISTRY"),
        env("LAFIYA_E2E_ATTESTATION_REGISTRY"),
    ) else {
        eprintln!("LAFIYA_E2E_* not set; skipping (run tests/e2e.sh)");
        return;
    };
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let metrics = Arc::new(Metrics::new());
    let rpc = RpcClient::new(
        rpc_url,
        vec![attester_registry.clone(), attestation_registry.clone()],
    );

    let ingestor = Ingestor {
        pool: pool.clone(),
        source: rpc.clone(),
        metrics: metrics.clone(),
        start_ledger: Some(start.parse().unwrap()),
        page_limit: 5,
    };
    while !ingestor.step().await.unwrap().caught_up {}

    let reconciler = Reconciler {
        pool: pool.clone(),
        rpc,
        metrics: metrics.clone(),
        attester_registry,
        attestation_registry,
        sample_size: 1_000,
        max_lag_ledgers: i64::MAX,
    };
    assert_eq!(
        reconciler.cycle().await.unwrap(),
        0,
        "reconciler mismatches"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(AppState { pool, metrics });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Attesters: status and region equal `get_attester_status`.
    let attesters = get_all(&base, "/attesters").await;
    let statuses: BTreeSet<&str> = attesters
        .iter()
        .map(|a| a["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        BTreeSet::from(["active", "removed", "suspended"]),
        "scenario should cover every attester status"
    );
    for a in &attesters {
        let chain = reconciler
            .chain_attester(a["address"].as_str().unwrap())
            .await
            .unwrap();
        let expected = match &chain {
            None => "removed",
            Some(c) if c.suspended => "suspended",
            Some(_) => "active",
        };
        assert_eq!(a["status"], expected, "{a}");
        assert_eq!(
            a["region"].as_str(),
            chain.as_ref().and_then(|c| c.region.as_deref()),
            "{a}"
        );
    }

    // Records: the current attestation from history equals `get_attestation`.
    let hashes: BTreeSet<String> = get_all(&base, "/attestations")
        .await
        .iter()
        .map(|a| a["record_hash"].as_str().unwrap().to_string())
        .collect();
    let mut saw_revoked = false;
    for hash in &hashes {
        let history = get_all(&base, &format!("/records/{hash}/history")).await;
        let mut current = None;
        for h in &history {
            current = match h["kind"].as_str().unwrap() {
                "revoked" => {
                    saw_revoked = true;
                    None
                }
                _ => Some(ChainAttestation {
                    attester: h["attester"].as_str().unwrap().into(),
                    timestamp: h["timestamp"].as_u64().unwrap(),
                }),
            };
        }
        let chain = reconciler.chain_attestation(hash).await.unwrap();
        assert_eq!(current, chain, "record {hash}");
    }
    assert!(saw_revoked, "scenario should include a revocation");
}

/// Full approve-claim cycle: the sandbox emits an approval and a claim against
/// the incentive pool; the indexer must surface both, and the payout must match
/// the approved amount.
#[tokio::test]
async fn indexes_approve_claim_cycle() {
    let (Some(rpc_url), Some(start), Some(attester_registry), Some(attestation_registry), Some(pool_contract)) = (
        env("LAFIYA_E2E_RPC_URL"),
        env("LAFIYA_E2E_START_LEDGER"),
        env("LAFIYA_E2E_ATTESTER_REGISTRY"),
        env("LAFIYA_E2E_ATTESTATION_REGISTRY"),
        env("LAFIYA_E2E_INCENTIVE_POOL"),
    ) else {
        eprintln!("LAFIYA_E2E_* not set; skipping (run tests/e2e.sh)");
        return;
    };
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let metrics = Arc::new(Metrics::new());
    let rpc = RpcClient::new(
        rpc_url,
        vec![
            attester_registry.clone(),
            attestation_registry.clone(),
            pool_contract.clone(),
        ],
    );

    let ingestor = Ingestor {
        pool: pool.clone(),
        source: rpc.clone(),
        metrics: metrics.clone(),
        start_ledger: Some(start.parse().unwrap()),
        page_limit: 5,
    };
    while !ingestor.step().await.unwrap().caught_up {}

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(AppState {
        pool: pool.clone(),
        metrics: metrics.clone(),
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The sandbox emits exactly one approval and one claim for the same work item.
    let work_items = get_all(&base, "/pool/work-items").await;
    assert!(
        !work_items.is_empty(),
        "sandbox should emit at least one pool work item"
    );
    let approved: Vec<&Value> = work_items
        .iter()
        .filter(|w| w["status"].as_str() == Some("approved"))
        .collect();
    assert!(
        !approved.is_empty(),
        "sandbox should emit an approval event"
    );

    let payouts = get_all(&base, "/pool/payouts").await;
    assert!(!payouts.is_empty(), "sandbox should emit a claim event");
    for payout in &payouts {
        let work_item_id = payout["work_item_id"].as_str().unwrap();
        let item = work_items
            .iter()
            .find(|w| w["work_item_id"].as_str() == Some(work_item_id))
            .unwrap_or_else(|| panic!("payout for unknown work item {work_item_id}"));
        assert_eq!(
            payout["amount"], item["amount"],
            "payout amount must equal approved amount for {work_item_id}"
        );
    }

    // Every API response carries the finality boundary.
    let body: Value = reqwest::get(format!("{base}/pool/payouts?limit=1"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        body["finalized_ledger"].as_u64().is_some(),
        "responses must expose finalized_ledger"
    );
}

/// Chaos: swap RPC providers mid-stream where the second provider serves a
/// forked history. The indexer must roll back to the common ancestor and
/// re-ingest deterministically.
#[tokio::test]
async fn reorg_rollback_is_deterministic() {
    let (Some(rpc_url), Some(start)) = (env("LAFIYA_E2E_RPC_URL"), env("LAFIYA_E2E_START_LEDGER"))
    else {
        eprintln!("LAFIYA_E2E_* not set; skipping (run tests/e2e.sh)");
        return;
    };
    let Some(pool) = common::test_pool().await else {
        return;
    };
    let metrics = Arc::new(Metrics::new());
    let contracts = vec![
        env("LAFIYA_E2E_ATTESTER_REGISTRY").unwrap_or_default(),
        env("LAFIYA_E2E_ATTESTATION_REGISTRY").unwrap_or_default(),
    ];
    let start_ledger: u32 = start.parse().unwrap();

    // First pass: ingest from the primary provider.
    let primary = RpcClient::new(rpc_url.clone(), contracts.clone());
    let ingestor = Ingestor {
        pool: pool.clone(),
        source: primary,
        metrics: metrics.clone(),
        start_ledger: Some(start_ledger),
        page_limit: 5,
    };
    while !ingestor.step().await.unwrap().caught_up {}
    let before = Store::new(pool.clone()).snapshot().await.unwrap();

    // Second pass: a provider whose history forks after `start_ledger`. The
    // ingest loop must detect the hash mismatch, roll back atomically, and
    // re-ingest. Re-running the same pass must yield identical state.
    let forked = RpcClient::new(
        format!("{rpc_url}?fork_after={start_ledger}"),
        contracts.clone(),
    );
    let ingestor = Ingestor {
        pool: pool.clone(),
        source: forked.clone(),
        metrics: metrics.clone(),
        start_ledger: Some(start_ledger),
        page_limit: 5,
    };
    while !ingestor.step().await.unwrap().caught_up {}
    let after_first = Store::new(pool.clone()).snapshot().await.unwrap();
    assert!(
        metrics.reorg_detected_total() > 0,
        "fork must be detected as a reorg"
    );

    let ingestor = Ingestor {
        pool: pool.clone(),
        source: forked,
        metrics: metrics.clone(),
        start_ledger: Some(start_ledger),
        page_limit: 5,
    };
    while !ingestor.step().await.unwrap().caught_up {}
    let after_second = Store::new(pool.clone()).snapshot().await.unwrap();
    assert_eq!(
        after_first, after_second,
        "re-ingesting the same fork must be idempotent"
    );
    assert_ne!(
        before, after_first,
        "forked history should change derived state"
    );
}
