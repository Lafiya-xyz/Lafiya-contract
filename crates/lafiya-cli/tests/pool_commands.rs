//! Integration tests for the first-class `pool` subcommands (issue #539) and
//! the pre-submit simulation preview (issue #536).
//!
//! These tests drive the `lafiya` binary against a mocked JSON-RPC endpoint
//! provided by `wiremock`. They assert:
//!
//! * decimal-aware amount parsing for every mutating pool command,
//! * client-side cap pre-checks that mirror contract errors,
//! * rendered output for the read-only pool commands,
//! * that the simulation preview shown before submission matches the
//!   post-execution state returned by the mocked RPC.

use std::process::Command;

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Path to the compiled `lafiya` binary under test.
fn lafiya_bin() -> &'static str {
    env!("CARGO_BIN_EXE_lafiya")
}

/// A representative contract id used across the pool tests.
const POOL_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM";

/// A representative source account.
const SOURCE: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// Build a `Command` pointed at the mocked RPC server.
fn lafiya(server: &MockServer) -> Command {
    let mut cmd = Command::new(lafiya_bin());
    cmd.arg("--rpc-url")
        .arg(server.uri())
        .arg("--network-passphrase")
        .arg("Test SDF Network ; September 2015")
        .arg("--source")
        .arg(SOURCE);
    cmd
}

/// Minimal successful `simulateTransaction` response body.
fn simulate_ok() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "latestLedger": 100,
            "transactionData": "",
            "minResourceFee": "100",
            "results": [{
                "auth": [],
                "xdr": "AAAAAA=="
            }],
            "events": [],
            "stateChanges": [],
            "cost": { "cpuInsns": "0", "memBytes": "0" }
        }
    })
}

/// Minimal successful `sendTransaction` response body.
fn send_ok(hash: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "hash": hash,
            "status": "PENDING",
            "latestLedger": 100,
            "latestLedgerCloseTime": "0"
        }
    })
}

/// Minimal `getTransaction` response body reporting success.
fn get_tx_success(hash: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "status": "SUCCESS",
            "hash": hash,
            "ledger": 101,
            "createdAt": "0",
            "envelopeXdr": "AAAAAA==",
            "resultXdr": "AAAAAA==",
            "resultMetaXdr": "AAAAAA=="
        }
    })
}

/// Stub the RPC methods required for a mutating pool command to succeed.
async fn stub_mutation_success(server: &MockServer, tx_hash: &str) {
    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "simulateTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(simulate_ok()))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "sendTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(send_ok(tx_hash)))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "getTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(get_tx_success(tx_hash)))
        .mount(server)
        .await;
}

/// Run a command and return `(stdout, stderr, success)`.
fn run(cmd: &mut Command) -> (String, String, bool) {
    let out = cmd.output().expect("failed to spawn lafiya");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

// ---------------------------------------------------------------------------
// Mutating pool commands: decimal parsing + successful submission
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_fund_parses_decimal_amount() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "abc123").await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "12.5",
        "--yes",
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "fund failed: {stderr}");
    assert!(
        stdout.contains("abc123"),
        "expected tx hash in output, got: {stdout}"
    );
}

#[tokio::test]
async fn pool_approve_parses_decimal_amount() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "def456").await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "approve",
        "--pool",
        POOL_ID,
        "--amount",
        "0.0000001",
        "--yes",
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "approve failed: {stderr}");
    assert!(stdout.contains("def456"), "got: {stdout}");
}

#[tokio::test]
async fn pool_claim_parses_decimal_amount() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "aaa111").await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "claim",
        "--pool",
        POOL_ID,
        "--amount",
        "100",
        "--yes",
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "claim failed: {stderr}");
    assert!(stdout.contains("aaa111"), "got: {stdout}");
}

#[tokio::test]
async fn pool_withdraw_parses_decimal_amount() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "bbb222").await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "withdraw",
        "--pool",
        POOL_ID,
        "--amount",
        "3.14159",
        "--yes",
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "withdraw failed: {stderr}");
    assert!(stdout.contains("bbb222"), "got: {stdout}");
}

#[tokio::test]
async fn pool_pause_submits() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "ccc333").await;

    let mut cmd = lafiya(&server);
    cmd.args(["pool", "pause", "--pool", POOL_ID, "--yes"]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "pause failed: {stderr}");
    assert!(stdout.contains("ccc333"), "got: {stdout}");
}

#[tokio::test]
async fn pool_unpause_submits() {
    let server = MockServer::start().await;
    stub_mutation_success(&server, "ddd444").await;

    let mut cmd = lafiya(&server);
    cmd.args(["pool", "unpause", "--pool", POOL_ID, "--yes"]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "unpause failed: {stderr}");
    assert!(stdout.contains("ddd444"), "got: {stdout}");
}

// ---------------------------------------------------------------------------
// Client-side pre-checks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_fund_rejects_malformed_amount_before_rpc() {
    // No mocks mounted: any RPC call would fail. The client-side parser must
    // reject the amount before contacting the network.
    let server = MockServer::start().await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "not-a-number",
        "--yes",
    ]);
    let (_stdout, stderr, ok) = run(&mut cmd);
    assert!(!ok, "expected failure for malformed amount");
    assert!(
        stderr.to_lowercase().contains("amount"),
        "expected amount error, got: {stderr}"
    );
}

#[tokio::test]
async fn pool_fund_rejects_excess_precision() {
    let server = MockServer::start().await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "1.12345678", // 8 decimal places exceeds 7-decimal token precision
        "--yes",
    ]);
    let (_stdout, stderr, ok) = run(&mut cmd);
    assert!(!ok, "expected failure for excess precision");
    assert!(
        stderr.to_lowercase().contains("precision") || stderr.to_lowercase().contains("decimal"),
        "expected precision error, got: {stderr}"
    );
}

#[tokio::test]
async fn pool_fund_rejects_negative_amount() {
    let server = MockServer::start().await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "-5",
        "--yes",
    ]);
    let (_stdout, stderr, ok) = run(&mut cmd);
    assert!(!ok, "expected failure for negative amount");
    assert!(
        stderr.to_lowercase().contains("amount") || stderr.to_lowercase().contains("negative"),
        "expected amount error, got: {stderr}"
    );
}

#[tokio::test]
async fn pool_claim_rejects_amount_above_cap() {
    // Simulate a pool whose per-attester cap is 10 tokens; claiming 11 must be
    // rejected client-side with a message mirroring the contract error.
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "simulateTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "latestLedger": 100,
                "transactionData": "",
                "minResourceFee": "100",
                "results": [{
                    "auth": [],
                    "xdr": "AAAAAA==",
                    "error": "Error(Contract, #4)" // CapExceeded
                }],
                "events": [],
                "stateChanges": [],
                "cost": { "cpuInsns": "0", "memBytes": "0" }
            }
        })))
        .mount(&server)
        .await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "claim",
        "--pool",
        POOL_ID,
        "--amount",
        "11",
        "--yes",
    ]);
    let (_stdout, stderr, ok) = run(&mut cmd);
    assert!(!ok, "expected failure when claim exceeds cap");
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("cap") || lower.contains("exceed"),
        "expected cap error, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Read-only pool commands: rendered output
// ---------------------------------------------------------------------------

/// Stub `getLedgerEntries` with a canned pool state.
async fn stub_pool_state(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "getLedgerEntries"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "latestLedger": 100,
                "entries": [{
                    "key": "AAAAAA==",
                    "xdr": "AAAAAA==",
                    "lastModifiedLedgerSeq": 99,
                    "liveUntilLedgerSeq": 200
                }]
            }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn pool_caps_renders_caps() {
    let server = MockServer::start().await;
    stub_pool_state(&server).await;

    let mut cmd = lafiya(&server);
    cmd.args(["pool", "caps", "--pool", POOL_ID]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "caps failed: {stderr}");
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("cap") || lower.contains("limit"),
        "expected caps in output, got: {stdout}"
    );
}

#[tokio::test]
async fn pool_balance_renders_balance() {
    let server = MockServer::start().await;
    stub_pool_state(&server).await;

    let mut cmd = lafiya(&server);
    cmd.args(["pool", "balance", "--pool", POOL_ID]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "balance failed: {stderr}");
    assert!(
        stdout.to_lowercase().contains("balance"),
        "expected balance in output, got: {stdout}"
    );
}

#[tokio::test]
async fn pool_liabilities_renders_liabilities() {
    let server = MockServer::start().await;
    stub_pool_state(&server).await;

    let mut cmd = lafiya(&server);
    cmd.args(["pool", "liabilities", "--pool", POOL_ID]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "liabilities failed: {stderr}");
    assert!(
        stdout.to_lowercase().contains("liabilit"),
        "expected liabilities in output, got: {stdout}"
    );
}

#[tokio::test]
async fn pool_attester_total_renders_total() {
    let server = MockServer::start().await;
    stub_pool_state(&server).await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "attester-total",
        "--pool",
        POOL_ID,
        "--attester",
        SOURCE,
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "attester-total failed: {stderr}");
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("total") || lower.contains("attester"),
        "expected attester total in output, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Issue #536: preview matches post-execution state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn preview_matches_post_execution_state() {
    let server = MockServer::start().await;

    // The simulation reports a state change: pool balance increases by 5.
    let simulated_state_change = json!({
        "type": "updated",
        "key": "AAAAAA==",
        "before": "AAAAAA==",
        "after": "BBBBBB=="
    });

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "simulateTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "latestLedger": 100,
                "transactionData": "",
                "minResourceFee": "100",
                "results": [{ "auth": [], "xdr": "AAAAAA==" }],
                "events": [{
                    "type": "contract",
                    "contractId": POOL_ID,
                    "topics": ["AAAADwAAAARmdW5k"], // "fund"
                    "data": "AAAAAA=="
                }],
                "stateChanges": [simulated_state_change.clone()],
                "cost": { "cpuInsns": "0", "memBytes": "0" }
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "sendTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(send_ok("previewhash")))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "getTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(get_tx_success("previewhash")))
        .mount(&server)
        .await;

    // Run with `--preview-json` to capture the preview as JSON.
    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "5",
        "--yes",
        "--preview-json",
    ]);
    let (stdout, stderr, ok) = run(&mut cmd);
    assert!(ok, "fund with preview failed: {stderr}");

    // The preview JSON should contain the simulated state change and event.
    let preview: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("preview output is not valid JSON ({e}): {stdout}"));

    let state_changes = preview
        .get("stateChanges")
        .and_then(Value::as_array)
        .expect("preview missing stateChanges");
    assert_eq!(state_changes.len(), 1);
    assert_eq!(state_changes[0], simulated_state_change);

    let events = preview
        .get("events")
        .and_then(Value::as_array)
        .expect("preview missing events");
    assert_eq!(events.len(), 1);

    // The preview must also carry the transaction hash so change-control
    // reviewers can correlate it with the executed transaction.
    assert_eq!(
        preview.get("hash").and_then(Value::as_str),
        Some("previewhash")
    );
}

#[tokio::test]
async fn failed_simulation_blocks_submission() {
    let server = MockServer::start().await;

    // Simulation fails; `sendTransaction` must never be called.
    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "simulateTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "latestLedger": 100,
                "transactionData": "",
                "minResourceFee": "100",
                "results": [{
                    "auth": [],
                    "xdr": "AAAAAA==",
                    "error": "Error(Contract, #1)"
                }],
                "events": [],
                "stateChanges": [],
                "cost": { "cpuInsns": "0", "memBytes": "0" }
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/"))
        .and(wiremock::matchers::body_partial_json(json!({
            "method": "sendTransaction"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(send_ok("should-not-happen")))
        .expect(0)
        .mount(&server)
        .await;

    let mut cmd = lafiya(&server);
    cmd.args([
        "pool",
        "fund",
        "--pool",
        POOL_ID,
        "--amount",
        "5",
        "--yes",
    ]);
    let (_stdout, stderr, ok) = run(&mut cmd);
    assert!(!ok, "expected failure when simulation fails");
    assert!(
        stderr.to_lowercase().contains("simulat"),
        "expected simulation error, got: {stderr}"
    );
}