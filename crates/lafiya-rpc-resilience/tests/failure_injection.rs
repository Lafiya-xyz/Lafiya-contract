//! Failure-injection scenarios for the RPC failover spike (issue #132,
//! `docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md`).
//!
//! Each test reproduces one named failure mode from the ADR's evaluation
//! and asserts both the final [`RecoveryResult`] and the exact number of
//! `submit`/`get_transaction` calls made to each provider -- the call
//! counts are what prove a transaction was never resubmitted while its
//! outcome was still ambiguous.

use lafiya_rpc_resilience::mock::{ScriptedProvider, Shared};
use lafiya_rpc_resilience::{
    FailoverClient, RecoveryLog, RecoveryResult, RetryPolicy, RpcError, RpcProvider, SubmitOutcome,
    TxState,
};

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_submit_rounds: 4,
        max_poll_rounds: 4,
        base_backoff: std::time::Duration::from_millis(1),
        max_backoff: std::time::Duration::from_millis(50),
    }
}

#[test]
fn timeout_before_send_is_safe_to_retry_and_succeeds() {
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Definite(RpcError::Timeout))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 100 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-timeout-before-send", &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 100,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        2,
        "a definite pre-send failure must be retried"
    );
    assert_eq!(
        handle.borrow().poll_calls,
        0,
        "no ambiguity here, so no polling was needed"
    );
}

#[test]
fn ambiguous_timeout_after_send_polls_instead_of_resubmitting() {
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::Accepted { ledger: 55 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-ambiguous-accepted", &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 55,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "an ambiguous outcome must never trigger a blind resubmission"
    );
    assert_eq!(handle.borrow().poll_calls, 1);
}

#[test]
fn ambiguous_timeout_after_send_polls_and_finds_rejection() {
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::Rejected {
                reason: "trustline missing".to_string(),
            })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-ambiguous-rejected", &mut log);

    assert_eq!(
        result,
        RecoveryResult::RejectedOnChain {
            reason: "trustline missing".to_string(),
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "still no blind resubmission"
    );
}

#[test]
fn rate_limit_backs_off_then_succeeds_on_the_same_provider() {
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Definite(RpcError::RateLimited {
                retry_after: Some(std::time::Duration::from_millis(5)),
            }))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 7 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-rate-limited", &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 7,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(handle.borrow().submit_calls, 2);
    assert!(
        log.lines().iter().any(|l| l.contains("rate limited")),
        "the recovery log must surface the rate-limit signal for operators"
    );
}

#[test]
fn primary_provider_down_fails_over_to_secondary_without_duplicate_submission() {
    let primary = Shared::new(
        ScriptedProvider::new("primary")
            .then_submit(SubmitOutcome::Definite(RpcError::ProviderUnavailable)),
    );
    let secondary = Shared::new(
        ScriptedProvider::new("secondary")
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 9000 })),
    );
    let primary_handle = primary.handle();
    let secondary_handle = secondary.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(primary), Box::new(secondary)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-failover", &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 9000,
            provider: "secondary".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        primary_handle.borrow().submit_calls,
        1,
        "primary is tried exactly once, not looped on"
    );
    assert_eq!(
        secondary_handle.borrow().submit_calls,
        1,
        "secondary must receive the transaction exactly once, never twice"
    );
}

#[test]
fn exhausting_submit_rounds_escalates_to_the_operator_as_not_submitted() {
    let node = Shared::new(ScriptedProvider::new("solo")); // every call falls through to Definite(ProviderUnavailable)
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(
        providers,
        RetryPolicy {
            max_submit_rounds: 3,
            ..policy()
        },
    );
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-always-down", &mut log);

    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::NotSubmitted,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        3,
        "must stop at the configured round budget, not loop forever"
    );
}

#[test]
fn exhausting_poll_rounds_escalates_to_the_operator_as_unknown() {
    let node = Shared::new(
        ScriptedProvider::new("solo").then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout)),
    );
    // No polls scripted: ScriptedProvider defaults every unscripted poll to Ok(Unknown),
    // modeling a provider whose retention window already dropped the hash.
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(
        providers,
        RetryPolicy {
            max_poll_rounds: 3,
            ..policy()
        },
    );
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery("tx-vanishes", &mut log);

    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::Unknown,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "still exactly one submission despite the unresolved poll"
    );
    assert_eq!(handle.borrow().poll_calls, 3);
}
