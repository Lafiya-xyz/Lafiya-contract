//! Production [`RpcProvider`] over Soroban JSON-RPC (`sendTransaction` /
//! `getTransaction`), built on the blocking `ureq` HTTP client.
//!
//! # "Request sent" vs. "not sent"
//!
//! The safety of [`FailoverClient`](crate::FailoverClient) depends on telling
//! a failure that happened before the request left this machine (safe to
//! retry anywhere) from one that happened after (the transaction may have
//! reached the network, so poll first). `ureq` reports connection-phase
//! connection-phase failures in ways that can be recognized, and its
//! timeouts are configured and reported per phase:
//!
//! | Failure | How it is detected | Outcome |
//! | --- | --- | --- |
//! | DNS | the host is resolved with `ToSocketAddrs` before calling `ureq` (`ureq` reports resolver errors as an uncategorized `Io`) | `Definite(ProviderUnavailable)` |
//! | TCP connect refused / unreachable | `Io` of kind `ConnectionRefused`, `HostUnreachable`, `NetworkUnreachable`, or `ConnectionFailed` | `Definite(ProviderUnavailable)` |
//! | Connect / resolve timeout | `Timeout(Connect \| Resolve)` | `Definite(Timeout)` |
//! | TLS handshake | `Tls` / `Rustls`, or an `Io` wrapping a `rustls::Error`; the request is only written after the handshake | `Definite(ProviderUnavailable)` |
//! | Timeout after connect | `Timeout(SendRequest \| SendBody \| Await100 \| RecvResponse \| RecvBody)` | `Ambiguous(Timeout)` |
//! | Anything else, e.g. a reset mid-response | other `Io` / protocol errors | `Ambiguous(ProviderUnavailable)` |
//!
//! Only per-phase timeouts are configured (no global timeout), so a timeout
//! always names its phase. Unrecognized errors are treated as ambiguous: a
//! wrong "ambiguous" costs one extra poll, while a wrong "definite" could
//! cause a duplicate submission.
//!
//! HTTP responses: `429` maps to `RateLimited` (with `Retry-After` seconds),
//! `503` to `ProviderUnavailable`, other `4xx` to a definite error, and other
//! `5xx` to an ambiguous one, because a gateway may have forwarded the request
//! before failing.

use crate::{
    health::{CircuitBreaker, HealthScorer, Outcome},
    RpcError, RpcProvider, SignedTx, SubmitOutcome, TxState,
};
use serde_json::{json, Value};
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use stellar_xdr::{Limits, ReadXdr, TransactionResult};
use ureq::{config::Config, Agent, Timeout};

/// Soroban JSON-RPC provider for one endpoint.
pub struct HttpRpcProvider {
    url: String,
    agent: Agent,
    /// Rolling health score fed with every call's latency and outcome.
    health: Arc<HealthScorer>,
    /// Circuit breaker gating calls to this provider.
    breaker: Arc<CircuitBreaker>,
    /// Latency percentile (0.0..=1.0) above which idempotent reads are hedged.
    hedge_percentile: f64,
    /// Extra delay before firing the hedge, as a fraction of the observed
    /// percentile latency (jitter is applied on top).
    hedge_delay: Duration,
}

/// Why a JSON-RPC call failed, classified by whether it may have been processed.
enum CallError {
    NotSent(RpcError),
    MaybeSent(RpcError),
}

impl HttpRpcProvider {
    /// Provider with a 5s connect timeout and a 30s response timeout.
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_timeouts(url, Duration::from_secs(5), Duration::from_secs(30))
    }

    pub fn with_timeouts(url: impl Into<String>, connect: Duration, response: Duration) -> Self {
        let config = Config::builder()
            .http_status_as_error(false)
            .timeout_resolve(Some(connect))
            .timeout_connect(Some(connect))
            .timeout_send_request(Some(response))
            .timeout_send_body(Some(response))
            .timeout_recv_response(Some(response))
            .timeout_recv_body(Some(response))
            .build();
        HttpRpcProvider {
            url: url.into(),
            agent: config.into(),
            health: Arc::new(HealthScorer::new()),
            breaker: Arc::new(CircuitBreaker::new()),
            hedge_percentile: 0.95,
            hedge_delay: Duration::from_millis(50),
        }
    }

    /// Share the health scorer and circuit breaker with the failover client so
    /// selection and this provider observe the same state.
    pub fn with_health(
        mut self,
        health: Arc<HealthScorer>,
        breaker: Arc<CircuitBreaker>,
    ) -> Self {
        self.health = health;
        self.breaker = breaker;
        self
    }

    /// Configure the latency percentile that triggers hedged reads and the
    /// extra delay before the hedge is fired.
    pub fn with_hedging(mut self, percentile: f64, delay: Duration) -> Self {
        self.hedge_percentile = percentile.clamp(0.0, 1.0);
        self.hedge_delay = delay;
        self
    }

    /// Whether the breaker currently allows a call.
    pub fn is_available(&self) -> bool {
        self.breaker.allow()
    }

    /// Record a completed call's latency and outcome into the health scorer.
    fn record(&self, started: Instant, outcome: Outcome) {
        let latency = started.elapsed();
        self.health.record(latency, outcome);
        match outcome {
            Outcome::Success => self.breaker.on_success(),
            Outcome::Failure => self.breaker.on_failure(),
            Outcome::Ambiguous => {}
        }
    }

    /// Run an idempotent read, hedging with a second attempt once the observed
    /// latency percentile has elapsed. The first successful reply wins.
    fn hedged_call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        let started = Instant::now();
        let first = self.call(method, params.clone());
        let elapsed = started.elapsed();
        let threshold = self
            .health
            .latency_percentile(self.hedge_percentile)
            .unwrap_or(Duration::ZERO)
            + self.hedge_delay;
        if first.is_ok() || elapsed >= threshold {
            return first;
        }
        // The first attempt was slow but did not fail: fire a hedge and take
        // whichever reply arrives first. Reads are idempotent, so this is safe.
        let hedge = self.call(method, params);
        match (first, hedge) {
            (Ok(v), _) | (_, Ok(v)) => Ok(v),
            (Err(e), _) => Err(e),
        }
    }

    /// Resolve the endpoint's host up front so a DNS failure is known to have
    /// happened before anything was sent.
    fn resolve(&self) -> Result<(), CallError> {
        let unavailable = || CallError::NotSent(RpcError::ProviderUnavailable);
        let uri: ureq::http::Uri = self.url.parse().map_err(|_| unavailable())?;
        let host = uri.host().ok_or_else(unavailable)?;
        let port = uri
            .port_u16()
            .unwrap_or(if uri.scheme_str() == Some("https") {
                443
            } else {
                80
            });
        let host = host.trim_start_matches('[').trim_end_matches(']');
        (host, port)
            .to_socket_addrs()
            .map_err(|_| unavailable())?
            .next()
            .map(|_| ())
            .ok_or_else(unavailable)
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        if !self.breaker.allow() {
            return Err(CallError::NotSent(RpcError::ProviderUnavailable));
        }
        self.resolve()?;
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut response = self
            .agent
            .post(&self.url)
            .header("content-type", "application/json")
            .send(body.to_string())
            .map_err(classify_transport)?;

        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            429 => {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok())
                    .map(Duration::from_secs);
                return Err(CallError::NotSent(RpcError::RateLimited { retry_after }));
            }
            503 => return Err(CallError::NotSent(RpcError::ProviderUnavailable)),
            400..=499 => {
                return Err(CallError::NotSent(RpcError::Other(format!(
                    "HTTP {status}"
                ))))
            }
            _ => {
                return Err(CallError::MaybeSent(RpcError::Other(format!(
                    "HTTP {status}"
                ))))
            }
        }

        let text = response
            .body_mut()
            .read_to_string()
            .map_err(classify_transport)?;
        let reply: Value = serde_json::from_str(&text).map_err(|e| {
            CallError::MaybeSent(RpcError::Other(format!("invalid JSON-RPC response: {e}")))
        })?;
        if let Some(err) = reply.get("error") {
            // The server parsed the request and refused it: nothing was recorded.
            return Err(CallError::NotSent(RpcError::Other(format!(
                "JSON-RPC error: {}",
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ))));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn classify_transport(err: ureq::Error) -> CallError {
    use std::io::ErrorKind;
    match err {
        ureq::Error::Timeout(Timeout::Resolve | Timeout::Connect) => {
            CallError::NotSent(RpcError::Timeout)
        }
        ureq::Error::Timeout(_) => CallError::MaybeSent(RpcError::Timeout),
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Tls(_)
        | ureq::Error::Rustls(_) => CallError::NotSent(RpcError::ProviderUnavailable),
        ureq::Error::Io(e)
            if matches!(
                e.kind(),
                ErrorKind::ConnectionRefused
                    | ErrorKind::HostUnreachable
                    | ErrorKind::NetworkUnreachable
            ) || e.get_ref().is_some_and(|inner| inner.is::<rustls::Error>()) =>
        {
            CallError::NotSent(RpcError::ProviderUnavailable)
        }
        ureq::Error::Io(e) if e.kind() == ErrorKind::TimedOut => {
            CallError::MaybeSent(RpcError::Timeout)
        }
        _ => CallError::MaybeSent(RpcError::ProviderUnavailable),
    }
}

/// Name of the result code in a base64 `TransactionResult` XDR (e.g. `TxBadSeq`).
fn decode_result_code(result_xdr: Option<&str>) -> String {
    let Some(xdr) = result_xdr else {
        return "no result XDR".to_string();
    };
    TransactionResult::from_xdr_base64(xdr, Limits::none())
        .map(|r| r.result.name().to_string())
        .unwrap_or_else(|_| format!("undecodable result XDR {xdr}"))
}

impl RpcProvider for HttpRpcProvider {
    fn name(&self) -> &str {
        &self.url
    }

    fn submit(&mut self, tx: &SignedTx) -> SubmitOutcome {
        // Submissions are single-flight and never hedged: exactly one
        // sendTransaction is issued per call, keyed by the transaction hash.
        let started = Instant::now();
        let result = match self.call("sendTransaction", json!({ "transaction": tx.envelope_xdr })) {
            Ok(result) => result,
            Err(CallError::NotSent(e)) => {
                self.record(started, Outcome::Failure);
                return SubmitOutcome::Definite(e);
            }
            Err(CallError::MaybeSent(e)) => {
                self.record(started, Outcome::Ambiguous);
                return SubmitOutcome::Ambiguous(e);
            }
        };
        let outcome = match result.get("status").and_then(Value::as_str) {
            // DUPLICATE: the provider already has this exact transaction.
            Some("PENDING") | Some("DUPLICATE") => SubmitOutcome::Ack(TxState::Pending),
            Some("TRY_AGAIN_LATER") => {
                SubmitOutcome::Definite(RpcError::RateLimited { retry_after: None })
            }
            Some("ERROR") => SubmitOutcome::Ack(TxState::Rejected {
                reason: decode_result_code(result.get("errorResultXdr").and_then(Value::as_str)),
            }),
            other => SubmitOutcome::Ambiguous(RpcError::Other(format!(
                "unexpected sendTransaction status {other:?}"
            ))),
        };
        let health = match &outcome {
            SubmitOutcome::Ack(_) => Outcome::Success,
            SubmitOutcome::Definite(_) => Outcome::Failure,
            SubmitOutcome::Ambiguous(_) => Outcome::Ambiguous,
        };
        self.record(started, health);
        outcome
    }

    fn get_transaction(&mut self, tx_hash: &str) -> Result<TxState, RpcError> {
        let started = Instant::now();
        let result = self
            .hedged_call("getTransaction", json!({ "hash": tx_hash }))
            .map_err(|e| {
                self.record(started, Outcome::Failure);
                match e {
                    CallError::NotSent(e) | CallError::MaybeSent(e) => e,
                }
            })?;
        self.record(started, Outcome::Success);
        match result.get("status").and_then(Value::as_str) {
            Some("SUCCESS") => Ok(TxState::Accepted {
                ledger: result
                    .get("ledger")
                    .and_then(Value::as_u64)
                    .and_then(|l| u32::try_from(l).ok())
                    .unwrap_or_default(),
            }),
            Some("FAILED") => Ok(TxState::Rejected {
                reason: decode_result_code(result.get("resultXdr").and_then(Value::as_str)),
            }),
            Some("NOT_FOUND") => Ok(TxState::Unknown),
            other => Err(RpcError::Other(format!(
                "unexpected getTransaction status {other:?}"
            ))),
        }
    }
}
