//! End-to-end: a scripted incident scenario (the event sequence a local
//! quickstart produces for each privileged action) goes through the rules
//! engine and a real HTTP webhook sink, and a mock webhook captures every
//! critical alert.
//!
//! `live_quickstart_scenario` runs the same check against a live quickstart
//! RPC; it is ignored by default. See docs/runbooks/watchdog.md.

use lafiya_watchdog::config::{Config, Heartbeat, RateRule, SinkConfig};
use lafiya_watchdog::rules::{self, Engine, HealthMonitor};
use lafiya_watchdog::{sinks, Event, Severity};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

/// Spawn a one-thread HTTP server that records each POST body.
fn mock_webhook() -> (String, mpsc::Receiver<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let _ = tx.send(serde_json::from_slice(&body).unwrap());
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .unwrap();
        }
    });
    (url, rx)
}

fn config(url: String) -> Config {
    Config {
        rpc_url: "http://localhost:8000/rpc".into(),
        contracts: vec!["CREG".into(), "CATT".into()],
        poll_interval_secs: 1,
        explorer_tx_url: None,
        release_manifests: vec![],
        known_wasm_hashes: vec!["aa".repeat(32)],
        rules: vec![],
        mass_enrollment: Some(RateRule {
            event: "attester_added".into(),
            max_count: 5,
            window_ledgers: 100,
            severity: Severity::High,
        }),
        heartbeat: Heartbeat::default(),
        sinks: vec![SinkConfig::Webhook {
            url,
            format: "generic".into(),
            min_severity: Some(Severity::Critical),
        }],
    }
}

fn ev(contract: &str, name: &str, ledger: u32, topics: &[&str]) -> Event {
    Event {
        ledger,
        contract: contract.into(),
        name: name.into(),
        topics: topics.iter().map(|t| t.to_string()).collect(),
        data: String::new(),
        tx_hash: None,
        closed_at: None,
    }
}

#[test]
fn every_critical_rule_reaches_the_webhook() {
    let (url, rx) = mock_webhook();
    let cfg = config(url);
    let mut engine = Engine::new(&cfg);
    let unknown = "bb".repeat(32);

    let scenario = [
        ev("CREG", "attester_added", 10, &["GA"]),
        ev("CREG", "admin_transferred", 11, &["GOLD", "GNEW"]),
        ev(
            "CATT",
            "attester_registry_repointed",
            12,
            &["CREG", "CEVIL"],
        ),
        ev("CREG", "upgraded", 13, &[&unknown]),
        ev("CREG", "multisig_signers_changed", 14, &[]),
        ev("CATT", "paused", 15, &["GADMIN"]),
    ];
    let mut alerts: Vec<_> = scenario.iter().flat_map(|e| engine.evaluate(e)).collect();
    // The instance later runs wasm no event announced.
    engine.observe_wasm("CATT", &"aa".repeat(32), 16);
    alerts.extend(engine.observe_wasm("CATT", &"cc".repeat(32), 17));
    // And the watchdog itself stalls.
    alerts.extend(HealthMonitor::default().check_lag(20, 200, 60));

    for alert in &alerts {
        assert!(sinks::dispatch(&cfg.sinks, alert).is_empty());
    }

    let mut received: Vec<String> = Vec::new();
    let expected_critical = alerts
        .iter()
        .filter(|a| a.severity == Severity::Critical)
        .count();
    for _ in 0..expected_critical {
        let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        received.push(body["kind"].as_str().unwrap().to_string());
    }
    for kind in [
        "admin_transferred",
        "attester_registry_repointed",
        "upgraded",
        rules::UNKNOWN_WASM_UPGRADE,
        "multisig_reconfigured",
        rules::WASM_CHANGED_WITHOUT_EVENT,
        rules::WATCHDOG_LAG,
    ] {
        assert!(
            received.iter().any(|k| k == kind),
            "missing {kind} in {received:?}"
        );
    }
    // High-severity pause alert was filtered by min_severity = critical.
    assert!(!received.iter().any(|k| k == "pause_toggled"));
    assert!(rx.try_recv().is_err());
}

/// Pool activity (approvals, claims, withdrawals) must raise the new
/// incentive-pool alerts, and a critical pool pause must reach the webhook.
#[test]
fn incentive_pool_scenario_reaches_the_webhook() {
    let (url, rx) = mock_webhook();
    let mut cfg = config(url);
    cfg.contracts = vec!["CPOOL".into(), "CREG".into()];
    cfg.rules = vec![
        RateRule {
            event: "pool_approved".into(),
            max_count: 3,
            window_ledgers: 10,
            severity: Severity::High,
        },
        RateRule {
            event: "pool_withdrawn".into(),
            max_count: 1,
            window_ledgers: 10,
            severity: Severity::High,
        },
    ];
    let mut engine = Engine::new(&cfg);

    let attester = "GA".to_string();
    let scenario = [
        // Attester is added, then removed and re-added shortly after.
        ev("CREG", "attester_added", 10, &["GA"]),
        ev("CREG", "attester_removed", 11, &["GA"]),
        ev("CREG", "attester_added", 12, &["GA"]),
        // A burst of approvals in the same window.
        ev("CPOOL", "pool_approved", 13, &[&attester, "1000"]),
        ev("CPOOL", "pool_approved", 14, &[&attester, "1000"]),
        ev("CPOOL", "pool_approved", 15, &[&attester, "1000"]),
        ev("CPOOL", "pool_approved", 16, &[&attester, "1000"]),
        // A claim by the recently re-added attester.
        ev("CPOOL", "pool_claimed", 17, &[&attester, "1000"]),
        // A large withdrawal.
        ev("CPOOL", "pool_withdrawn", 18, &["1000000"]),
        // The pool is paused.
        ev("CPOOL", "pool_paused", 19, &["GADMIN"]),
    ];
    let alerts: Vec<_> = scenario.iter().flat_map(|e| engine.evaluate(e)).collect();

    for alert in &alerts {
        assert!(sinks::dispatch(&cfg.sinks, alert).is_empty());
    }

    let kinds: Vec<String> = alerts.iter().map(|a| a.kind.clone()).collect();
    for kind in [
        rules::POOL_APPROVAL_BURST,
        rules::POOL_LARGE_WITHDRAWAL,
        rules::POOL_CLAIM_BY_READDED_ATTESTER,
        rules::POOL_PAUSED,
    ] {
        assert!(
            kinds.iter().any(|k| k == kind),
            "missing {kind} in {kinds:?}"
        );
    }

    // Only the critical pool pause is forwarded to the webhook.
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(body["kind"].as_str().unwrap(), rules::POOL_PAUSED);
    assert!(rx.try_recv().is_err());
}

/// Requires a running quickstart (`stellar container start local`) with the
/// Lafiya contracts deployed and the scenario in docs/runbooks/watchdog.md
/// executed. Set `LAFIYA_WATCHDOG_RPC` and `LAFIYA_WATCHDOG_CONTRACTS`
/// (comma separated) and run with `--ignored`.
#[test]
#[ignore]
fn live_quickstart_scenario() {
    let rpc_url = std::env::var("LAFIYA_WATCHDOG_RPC").expect("LAFIYA_WATCHDOG_RPC");
    let contracts: Vec<String> = std::env::var("LAFIYA_WATCHDOG_CONTRACTS")
        .expect("LAFIYA_WATCHDOG_CONTRACTS")
        .split(',')
        .map(str::to_string)
        .collect();
    let (url, rx) = mock_webhook();
    let mut cfg = config(url);
    cfg.rpc_url = rpc_url;
    cfg.contracts = contracts;
    let rpc = lafiya_watchdog::rpc::Rpc::new(&cfg.rpc_url);
    let (events, _) = rpc.events(&cfg.contracts, 1, None).unwrap();
    let mut engine = Engine::new(&cfg);
    for alert in events.iter().flat_map(|e| engine.evaluate(e)) {
        sinks::dispatch(&cfg.sinks, &alert);
    }
    let kinds: Vec<String> = rx
        .try_iter()
        .map(|b| b["kind"].as_str().unwrap().to_string())
        .collect();
    for kind in [
        "admin_transferred",
        "upgraded",
        "attester_registry_repointed",
    ] {
        assert!(
            kinds.iter().any(|k| k == kind),
            "missing {kind} in {kinds:?}"
        );
    }
}
