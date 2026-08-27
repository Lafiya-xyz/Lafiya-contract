//! Reference prototype for RPC provider failover, retry classification, and
//! transaction-recovery polling produced for the RPC failover spike (see
//! `docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md`).
//!
//! # Problem this models
//!
//! The CLI and Bash scripts submit a signed transaction to a single
//! configured RPC endpoint and treat "the HTTP call failed" as "the
//! transaction did not happen." That assumption is false for a timeout,
//! connection reset, or 5xx: the request may have reached the network
//! before the response was lost. Blindly resubmitting a *new* transaction
//! in that state risks a duplicate on-chain effect (e.g. adding the same
//! attester twice, or -- for a differently-shaped future operation -- a
//! double-spend). Blindly giving up risks losing an operation that actually
//! succeeded.
//!
//! This crate is not a Soroban RPC client. It is a small, dependency-free
//! state machine that a real client can wrap: it classifies a submit
//! failure as safe-to-retry or must-poll-first, and it implements the
//! poll-before-retry recovery loop across an ordered list of providers.
//! [`mock`] provides a scriptable fake provider so the exact failure
//! sequences described in the ADR (timeout-before-send, ambiguous
//! timeout-after-send, rate limiting, and a hard-down primary) can be
//! reproduced deterministically in `tests/failure_injection.rs` and
//! `examples/failure_injection_demo.rs` without a network or a live RPC
//! endpoint.

use std::fmt;
use std::time::Duration;

pub mod mock;

/// Where a transaction currently stands, as observed via a status query
/// (Soroban RPC `getTransaction`) rather than assumed from a submit call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxState {
    /// Never submitted to any provider.
    NotSubmitted,
    /// Sent to a provider; outcome not yet confirmed by polling.
    Submitted,
    /// Provider accepted it into its queue; still waiting on ledger close.
    Pending,
    /// A ledger closed with this transaction included and successful.
    Accepted { ledger: u32 },
    /// A ledger closed with this transaction included but it failed on-chain.
    Rejected { reason: String },
    /// This provider has no record of the hash (never seen it, or it has
    /// aged out of the provider's retention window).
    Unknown,
}

/// An RPC-level failure. Distinct from an on-chain rejection: this is about
/// whether a response was obtained at all, not what the response said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// No response before the client-side deadline. The request may or may
    /// not have reached the provider.
    Timeout,
    /// Provider returned 429 / a rate-limit signal before doing any work.
    RateLimited { retry_after: Option<Duration> },
    /// Connection refused / DNS failure / 5xx health-check failure: this
    /// provider did not process the request at all.
    ProviderUnavailable,
    /// Any other RPC-layer error, carrying the provider's message.
    Other(String),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Timeout => write!(f, "timeout"),
            RpcError::RateLimited {
                retry_after: Some(d),
            } => write!(f, "rate limited, retry after {d:?}"),
            RpcError::RateLimited { retry_after: None } => write!(f, "rate limited"),
            RpcError::ProviderUnavailable => write!(f, "provider unavailable"),
            RpcError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// The result of one `submit` call against one provider.
///
/// The three-way split -- acknowledged / definite failure / ambiguous
/// failure -- is the load-bearing distinction in this whole model. Collapsing
/// "definite" and "ambiguous" into one generic `Err` is exactly the bug this
/// prototype exists to avoid: it is what makes blind retry-on-any-error
/// unsafe in the current CLI and scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The provider gave a definite state for the submission itself.
    Ack(TxState),
    /// A pre-send or immediately-rejected-by-provider failure: nothing was
    /// forwarded to the network. Safe to retry (same or a new transaction).
    Definite(RpcError),
    /// The request may have reached the network before the response was
    /// lost. Whether the transaction was actually recorded is unknown until
    /// polled by hash.
    Ambiguous(RpcError),
}

/// What a caller is allowed to do next after a given [`SubmitOutcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// Nothing was recorded on-chain. Resubmitting (the same signed
    /// envelope, or a freshly built one) is safe.
    SafeToRetry,
    /// Outcome unknown. Poll by hash before resubmitting or building a new
    /// transaction -- resubmitting blindly here is the duplicate-submission
    /// risk called out in the spike.
    MustPollFirst,
    /// The chain already has a final verdict for this exact transaction.
    /// Do not resubmit it (its sequence number is now consumed either way).
    DoNotRetrySameTx,
    /// Already resolved successfully. No retry applies.
    NoRetryNeeded,
}

/// Classify a single submit outcome. Pure and total: every [`SubmitOutcome`]
/// maps to exactly one [`RetryClass`]. This is the "safe vs. unsafe retry
/// classes" deliverable from the spike, expressed as code instead of prose
/// so it can't drift from the recovery loop that implements it below.
pub fn classify(outcome: &SubmitOutcome) -> RetryClass {
    match outcome {
        SubmitOutcome::Definite(_) => RetryClass::SafeToRetry,
        SubmitOutcome::Ambiguous(_) => RetryClass::MustPollFirst,
        SubmitOutcome::Ack(TxState::Rejected { .. }) => RetryClass::DoNotRetrySameTx,
        SubmitOutcome::Ack(_) => RetryClass::NoRetryNeeded,
    }
}

/// A Soroban RPC provider, reduced to the two calls this model cares about.
/// A real implementation wraps an HTTP client and a provider's base URL; the
/// [`mock::ScriptedProvider`] wraps a scripted sequence of outcomes instead.
pub trait RpcProvider {
    /// Human-readable identifier used in logs (e.g. a provider's hostname).
    fn name(&self) -> &str;

    /// Attempt to submit the transaction with the given hash. `tx_hash` is
    /// the hash of the already-signed envelope, computed locally -- it does
    /// not change on retry, which is what makes polling by hash meaningful.
    fn submit(&mut self, tx_hash: &str) -> SubmitOutcome;

    /// Query the current on-chain state of a transaction by hash.
    fn get_transaction(&mut self, tx_hash: &str) -> Result<TxState, RpcError>;
}

/// Exponential backoff, capped at `max`. `attempt` is 1-based.
///
/// No jitter is applied here -- this prototype is deterministic on purpose
/// so it can be asserted against in tests. A production client should add
/// jitter (e.g. decorrelated jitter) before using this against a shared
/// provider from many concurrent callers, to avoid a synchronized retry
/// stampede after a provider-wide outage.
pub fn backoff_schedule(attempt: u32, base: Duration, max: Duration) -> Duration {
    let attempt = attempt.max(1);
    let factor = 1u32.checked_shl(attempt - 1).unwrap_or(u32::MAX);
    base.checked_mul(factor).unwrap_or(max).min(max)
}

/// Tunables for [`FailoverClient`]. Defaults are deliberately conservative
/// for an admin CLI making low-frequency, high-consequence calls, not a
/// high-throughput indexer.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Submit rounds to attempt (across all providers, round-robin) before a
    /// [`RpcError::Timeout`]/[`RpcError::ProviderUnavailable`] chain is
    /// treated as exhausted and handed back to the operator.
    pub max_submit_rounds: u32,
    /// Poll rounds (each round queries every provider once) to attempt
    /// before an ambiguous outcome is treated as still-unresolved and handed
    /// back to the operator.
    pub max_poll_rounds: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_submit_rounds: 3,
            max_poll_rounds: 5,
            base_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(8),
        }
    }
}

/// Human-readable trace of a recovery run, for operator reproducibility: the
/// same sequence a real client would log is available here as plain lines,
/// so a failure-injection run's log can be diffed against the runbook in
/// `docs/runbooks/rpc-outage-recovery.md`.
#[derive(Debug, Default, Clone)]
pub struct RecoveryLog(Vec<String>);

impl RecoveryLog {
    pub fn new() -> Self {
        RecoveryLog(Vec::new())
    }

    pub fn record(&mut self, line: impl Into<String>) {
        self.0.push(line.into());
    }

    pub fn lines(&self) -> &[String] {
        &self.0
    }
}

/// Final outcome of a recovery run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResult {
    Accepted {
        ledger: u32,
        provider: String,
        submit_attempts: u32,
    },
    RejectedOnChain {
        reason: String,
    },
    /// Every retry/poll budget was spent without a final verdict. This is
    /// the escalate-to-operator case; `last_known` is what to hand the
    /// runbook.
    ExhaustedNeedsOperator {
        last_known: TxState,
    },
}

/// Round-robins submission across an ordered list of providers and, on any
/// ambiguous or queued outcome, polls by hash (never resubmits blindly)
/// until a final verdict is reached or the retry policy is exhausted.
pub struct FailoverClient {
    providers: Vec<Box<dyn RpcProvider>>,
    policy: RetryPolicy,
}

impl FailoverClient {
    pub fn new(providers: Vec<Box<dyn RpcProvider>>, policy: RetryPolicy) -> Self {
        assert!(
            !providers.is_empty(),
            "FailoverClient requires at least one RPC provider"
        );
        FailoverClient { providers, policy }
    }

    /// Submit `tx_hash` and drive it to a final verdict, failing over
    /// between providers on definite failures and polling (never blind
    /// resubmitting) on ambiguous ones.
    pub fn submit_with_recovery(&mut self, tx_hash: &str, log: &mut RecoveryLog) -> RecoveryResult {
        let provider_count = self.providers.len();
        let mut submit_round: u32 = 0;

        loop {
            let idx = (submit_round as usize) % provider_count;
            submit_round += 1;

            let outcome = {
                let provider = &mut self.providers[idx];
                log.record(format!(
                    "round {submit_round}: submit {tx_hash} via {}",
                    provider.name()
                ));
                provider.submit(tx_hash)
            };

            match outcome {
                SubmitOutcome::Ack(TxState::Accepted { ledger }) => {
                    let provider = self.providers[idx].name().to_string();
                    log.record(format!(
                        "{provider} accepted immediately at ledger {ledger}"
                    ));
                    return RecoveryResult::Accepted {
                        ledger,
                        provider,
                        submit_attempts: submit_round,
                    };
                }
                SubmitOutcome::Ack(TxState::Rejected { reason }) => {
                    log.record(format!(
                        "rejected on submit ({reason}); do not retry this transaction"
                    ));
                    return RecoveryResult::RejectedOnChain { reason };
                }
                SubmitOutcome::Ack(_queued) => {
                    log.record(
                        "provider queued the transaction; polling for a final verdict".to_string(),
                    );
                    return self.poll_until_resolved(tx_hash, submit_round, log);
                }
                SubmitOutcome::Ambiguous(err) => {
                    log.record(format!(
                        "ambiguous failure ({err}) -- outcome unknown, polling before any retry"
                    ));
                    return self.poll_until_resolved(tx_hash, submit_round, log);
                }
                SubmitOutcome::Definite(err) => {
                    let backoff = backoff_schedule(
                        submit_round,
                        self.policy.base_backoff,
                        self.policy.max_backoff,
                    );
                    log.record(format!(
                        "definite failure ({err}) -- nothing recorded, safe to retry after {backoff:?}"
                    ));
                    if submit_round >= self.policy.max_submit_rounds {
                        log.record("submit rounds exhausted".to_string());
                        return RecoveryResult::ExhaustedNeedsOperator {
                            last_known: TxState::NotSubmitted,
                        };
                    }
                }
            }
        }
    }

    fn poll_until_resolved(
        &mut self,
        tx_hash: &str,
        mut attempts: u32,
        log: &mut RecoveryLog,
    ) -> RecoveryResult {
        let provider_count = self.providers.len();

        for poll_round in 1..=self.policy.max_poll_rounds {
            attempts += 1;

            for i in 0..provider_count {
                let provider = &mut self.providers[i];
                match provider.get_transaction(tx_hash) {
                    Ok(TxState::Accepted { ledger }) => {
                        let name = provider.name().to_string();
                        log.record(format!(
                            "poll {poll_round} via {name}: accepted at ledger {ledger}"
                        ));
                        return RecoveryResult::Accepted {
                            ledger,
                            provider: name,
                            submit_attempts: attempts,
                        };
                    }
                    Ok(TxState::Rejected { reason }) => {
                        log.record(format!(
                            "poll {poll_round} via {}: rejected on-chain ({reason})",
                            provider.name()
                        ));
                        return RecoveryResult::RejectedOnChain { reason };
                    }
                    Ok(TxState::Pending) | Ok(TxState::Submitted) => {
                        log.record(format!(
                            "poll {poll_round} via {}: still pending",
                            provider.name()
                        ));
                    }
                    Ok(TxState::Unknown) | Ok(TxState::NotSubmitted) => {
                        log.record(format!(
                            "poll {poll_round} via {}: hash unknown to this provider",
                            provider.name()
                        ));
                    }
                    Err(err) => {
                        log.record(format!(
                            "poll {poll_round} via {}: error ({err})",
                            provider.name()
                        ));
                    }
                }
            }

            let backoff = backoff_schedule(
                poll_round,
                self.policy.base_backoff,
                self.policy.max_backoff,
            );
            log.record(format!("backing off {backoff:?} before next poll round"));
        }

        log.record(
            "poll rounds exhausted without a final verdict -- escalate to operator".to_string(),
        );
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        assert_eq!(backoff_schedule(1, base, max), Duration::from_millis(100));
        assert_eq!(backoff_schedule(2, base, max), Duration::from_millis(200));
        assert_eq!(backoff_schedule(3, base, max), Duration::from_millis(400));
        assert_eq!(backoff_schedule(6, base, max), Duration::from_millis(3200));
        assert_eq!(
            backoff_schedule(7, base, max),
            max,
            "must cap instead of overflowing"
        );
        assert_eq!(
            backoff_schedule(20, base, max),
            max,
            "large attempts must still cap, not overflow"
        );
    }

    #[test]
    fn classify_matches_the_documented_retry_classes() {
        assert_eq!(
            classify(&SubmitOutcome::Definite(RpcError::Timeout)),
            RetryClass::SafeToRetry
        );
        assert_eq!(
            classify(&SubmitOutcome::Ambiguous(RpcError::Timeout)),
            RetryClass::MustPollFirst
        );
        assert_eq!(
            classify(&SubmitOutcome::Ack(TxState::Rejected {
                reason: "bad auth".into()
            })),
            RetryClass::DoNotRetrySameTx
        );
        assert_eq!(
            classify(&SubmitOutcome::Ack(TxState::Accepted { ledger: 42 })),
            RetryClass::NoRetryNeeded
        );
    }
}
