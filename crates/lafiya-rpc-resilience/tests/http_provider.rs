//! Reproduces every failure mode classified by `HttpRpcProvider` against a
//! wiremock Soroban RPC (plus raw sockets for connection-level faults).

use lafiya_rpc_resilience::http::HttpRpcProvider;
use lafiya_rpc_resilience::{
    FailoverClient, RecoveryLog, RecoveryResult, RetryPolicy, RpcError, RpcProvider, SignedTx,
    SubmitOutcome, TxState,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;
use stellar_xdr::{
    Limits, TransactionResult, TransactionResultExt, TransactionResultResult, WriteXdr,
};
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HASH: &str = "abababababababababababababababababababababababababababababababab";

fn tx() -> SignedTx {
    SignedTx::new(HASH, "AAAA")
}

fn provider(url: &str) -> HttpRpcProvider {
    HttpRpcProvider::with_timeouts(url, Duration::from_millis(500), Duration::from_millis(500))
}

fn rpc_result(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

async fn server_replying(rpc_method: &str, response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": rpc_method })))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn submit(url: String) -> SubmitOutcome {
    tokio::task::spawn_blocking(move || provider(&url).submit(&tx()))
        .await
        .unwrap()
}

async fn get_tx(url: String) -> Result<TxState, RpcError> {
    tokio::task::spawn_blocking(move || provider(&url).get_transaction(HASH))
        .await
        .unwrap()
}

async fn submit_replying(response: ResponseTemplate) -> SubmitOutcome {
    let server = server_replying("sendTransaction", response).await;
    submit(server.uri()).await
}

fn bad_seq_result_xdr() -> String {
    TransactionResult {
        fee_charged: 100,
        result: TransactionResultResult::TxBadSeq,
        ext: TransactionResultExt::V0,
    }
    .to_xdr_base64(Limits::none())
    .unwrap()
}

#[tokio::test]
async fn pending_and_duplicate_are_acked_as_pending() {
    for status in ["PENDING", "DUPLICATE"] {
        let outcome = submit_replying(rpc_result(json!({ "status": status, "hash": HASH }))).await;
        assert_eq!(outcome, SubmitOutcome::Ack(TxState::Pending), "{status}");
    }
}

#[tokio::test]
async fn try_again_later_is_definite_rate_limit() {
    let outcome = submit_replying(rpc_result(json!({ "status": "TRY_AGAIN_LATER" }))).await;
    assert_eq!(
        outcome,
        SubmitOutcome::Definite(RpcError::RateLimited { retry_after: None })
    );
}

#[tokio::test]
async fn error_status_is_rejected_with_decoded_result_code() {
    let outcome = submit_replying(rpc_result(
        json!({ "status": "ERROR", "errorResultXdr": bad_seq_result_xdr() }),
    ))
    .await;
    assert_eq!(
        outcome,
        SubmitOutcome::Ack(TxState::Rejected {
            reason: "TxBadSeq".into()
        })
    );
}

#[tokio::test]
async fn http_429_carries_retry_after() {
    let outcome =
        submit_replying(ResponseTemplate::new(429).insert_header("Retry-After", "7")).await;
    assert_eq!(
        outcome,
        SubmitOutcome::Definite(RpcError::RateLimited {
            retry_after: Some(Duration::from_secs(7))
        })
    );
}

#[tokio::test]
async fn http_503_is_definite_and_other_5xx_is_ambiguous() {
    assert_eq!(
        submit_replying(ResponseTemplate::new(503)).await,
        SubmitOutcome::Definite(RpcError::ProviderUnavailable)
    );
    assert!(matches!(
        submit_replying(ResponseTemplate::new(502)).await,
        SubmitOutcome::Ambiguous(_)
    ));
}

#[tokio::test]
async fn json_rpc_error_is_definite() {
    let outcome = submit_replying(ResponseTemplate::new(200).set_body_json(
        json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "invalid params"}}),
    ))
    .await;
    assert!(
        matches!(&outcome, SubmitOutcome::Definite(RpcError::Other(m)) if m.contains("invalid params")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn timeout_after_request_was_written_is_ambiguous() {
    let outcome = submit_replying(
        rpc_result(json!({ "status": "PENDING" })).set_delay(Duration::from_secs(3)),
    )
    .await;
    assert_eq!(outcome, SubmitOutcome::Ambiguous(RpcError::Timeout));
}

#[tokio::test]
async fn connection_refused_is_definite() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // Listener dropped: nothing accepts on this port.
    let outcome = submit(format!("http://127.0.0.1:{port}")).await;
    assert_eq!(
        outcome,
        SubmitOutcome::Definite(RpcError::ProviderUnavailable)
    );
}

#[tokio::test]
async fn dns_failure_is_definite() {
    let outcome = submit("http://lafiya-rpc.invalid".into()).await;
    assert_eq!(
        outcome,
        SubmitOutcome::Definite(RpcError::ProviderUnavailable)
    );
}

#[tokio::test]
async fn tls_failure_is_definite() {
    // An https request to a plain-HTTP server fails in the TLS handshake.
    let server = MockServer::start().await;
    let url = server.uri().replace("http://", "https://");
    let outcome = submit(url).await;
    assert_eq!(
        outcome,
        SubmitOutcome::Definite(RpcError::ProviderUnavailable)
    );
}

#[tokio::test]
async fn reset_mid_response_is_ambiguous() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        // Promise 100 bytes, send one, then drop the connection.
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{");
    });
    let outcome = submit(url).await;
    assert!(
        matches!(outcome, SubmitOutcome::Ambiguous(_)),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn get_transaction_maps_statuses() {
    let server = server_replying(
        "getTransaction",
        rpc_result(json!({ "status": "SUCCESS", "ledger": 1234 })),
    )
    .await;
    assert_eq!(
        get_tx(server.uri()).await,
        Ok(TxState::Accepted { ledger: 1234 })
    );

    let server = server_replying(
        "getTransaction",
        rpc_result(json!({ "status": "FAILED", "resultXdr": bad_seq_result_xdr() })),
    )
    .await;
    assert_eq!(
        get_tx(server.uri()).await,
        Ok(TxState::Rejected {
            reason: "TxBadSeq".into()
        })
    );

    let server = server_replying(
        "getTransaction",
        rpc_result(json!({ "status": "NOT_FOUND" })),
    )
    .await;
    assert_eq!(get_tx(server.uri()).await, Ok(TxState::Unknown));
}

#[tokio::test]
async fn failover_client_recovers_through_secondary_when_primary_is_down() {
    let secondary = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "sendTransaction" })))
        .respond_with(rpc_result(json!({ "status": "PENDING" })))
        .mount(&secondary)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "getTransaction" })))
        .respond_with(rpc_result(json!({ "status": "SUCCESS", "ledger": 77 })))
        .mount(&secondary)
        .await;
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let primary = format!("http://127.0.0.1:{port}");
    let secondary = secondary.uri();

    let (result, log) = tokio::task::spawn_blocking(move || {
        let providers: Vec<Box<dyn RpcProvider>> =
            vec![Box::new(provider(&primary)), Box::new(provider(&secondary))];
        let mut client = FailoverClient::new(providers, RetryPolicy::default());
        let mut log = RecoveryLog::new();
        (client.submit_with_recovery(&tx(), &mut log), log)
    })
    .await
    .unwrap();

    assert!(
        matches!(result, RecoveryResult::Accepted { ledger: 77, .. }),
        "{result:?}\n{:#?}",
        log.lines()
    );
}
