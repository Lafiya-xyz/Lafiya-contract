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
    FailoverClient, RecoveryLog, RecoveryResult, RestoreFootprint, RetryPolicy, RpcError,
    RpcProvider, SignedTx, SubmitOutcome, TxState,
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

    let result =
        client.submit_with_recovery(&SignedTx::new("tx-timeout-before-send", ""), &mut log);

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

    let result = client.submit_with_recovery(&SignedTx::new("tx-ambiguous-accepted", ""), &mut log);

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

    let result = client.submit_with_recovery(&SignedTx::new("tx-ambiguous-rejected", ""), &mut log);

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

    let result = client.submit_with_recovery(&SignedTx::new("tx-rate-limited", ""), &mut log);

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

    let result = client.submit_with_recovery(&SignedTx::new("tx-failover", ""), &mut log);

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

    let result = client.submit_with_recovery(&SignedTx::new("tx-always-down", ""), &mut log);

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
fn drop_before_send_is_reconciled_as_not_submitted_without_resubmission() {
    // The request never reaches the node: the intent is persisted, but the
    // hash is unknown to the chain. Reconciliation must prove the transaction
    // never landed and surface a precise NotSubmitted state.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::NotSubmitted)),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery(&SignedTx::new("tx-drop-before-send", ""), &mut log);

    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::NotSubmitted,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "a dropped-before-send intent must not be blindly resubmitted"
    );
    assert_eq!(
        handle.borrow().poll_calls,
        1,
        "reconciliation must query getTransaction for the persisted intent"
    );
}

#[test]
fn drop_after_send_is_reconciled_to_accepted_without_duplicate_submission() {
    // The node ingested the transaction but the ack was lost. The persisted
    // intent's hash must be reconciled via getTransaction to a final state.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::Unknown))
            .then_poll(Ok(TxState::Accepted { ledger: 4242 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery(&SignedTx::new("tx-drop-after-send", ""), &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 4242,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "the transaction already landed; resubmitting would duplicate it"
    );
    assert_eq!(
        handle.borrow().poll_calls,
        2,
        "reconciliation must keep polling until a final state is observed"
    );
}

#[test]
fn slow_ingest_is_reconciled_to_accepted_without_duplicate_submission() {
    // The node is slow to ingest: several polls return Unknown before the
    // transaction becomes visible. The client must wait it out, not resubmit.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::Unknown))
            .then_poll(Ok(TxState::Unknown))
            .then_poll(Ok(TxState::Accepted { ledger: 77 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_recovery(&SignedTx::new("tx-slow-ingest", ""), &mut log);

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 77,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "slow ingest must never trigger a duplicate submission"
    );
    assert_eq!(
        handle.borrow().poll_calls,
        3,
        "reconciliation must poll through the slow-ingest window"
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

    let result = client.submit_with_recovery(&SignedTx::new("tx-vanishes", ""), &mut log);

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

#[test]
fn archived_entries_trigger_restore_before_the_real_transaction() {
    // Simulation reports archived persistent entries (e.g. an old attestation
    // card). The client must submit a restore transaction first, then the real
    // transaction, without duplicating either.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_restore(SubmitOutcome::Ack(TxState::Accepted { ledger: 10 }))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 11 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let footprint = RestoreFootprint::new(vec!["attestation:card-1".to_string()]);
    let result = client.submit_with_restore(
        &SignedTx::new("tx-after-restore", ""),
        &footprint,
        &mut log,
    );

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 11,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().restore_calls,
        1,
        "the restore footprint must be submitted exactly once"
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "the real transaction is submitted exactly once, after restoration"
    );
    assert!(
        log.lines().iter().any(|l| l.contains("restore")),
        "the recovery log must record the restore step for operators"
    );
}

#[test]
fn restore_failure_escalates_without_submitting_the_real_transaction() {
    // If restoration cannot be confirmed, the real transaction must not be
    // submitted: it would fail against still-archived entries.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_restore(SubmitOutcome::Definite(RpcError::ProviderUnavailable)),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(
        providers,
        RetryPolicy {
            max_submit_rounds: 2,
            ..policy()
        },
    );
    let mut log = RecoveryLog::new();

    let footprint = RestoreFootprint::new(vec!["work_item:wo-9".to_string()]);
    let result = client.submit_with_restore(
        &SignedTx::new("tx-blocked-by-archive", ""),
        &footprint,
        &mut log,
    );

    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::NotSubmitted,
        }
    );
    assert_eq!(
        handle.borrow().restore_calls,
        2,
        "restore retries must respect the configured round budget"
    );
    assert_eq!(
        handle.borrow().submit_calls,
        0,
        "the real transaction must not be submitted while entries stay archived"
    );
}

#[test]
fn ambiguous_restore_is_reconciled_without_duplicate_restore() {
    // The restore transaction's ack is lost. Reconciliation must prove it
    // landed via getTransaction rather than blindly resubmitting it.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_restore(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_poll(Ok(TxState::Accepted { ledger: 20 }))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 21 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let footprint = RestoreFootprint::new(vec!["attester_info:att-3".to_string()]);
    let result = client.submit_with_restore(
        &SignedTx::new("tx-after-ambiguous-restore", ""),
        &footprint,
        &mut log,
    );

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 21,
            provider: "solo".to_string(),
            submit_attempts: 2,
        }
    );
    assert_eq!(
        handle.borrow().restore_calls,
        1,
        "an ambiguous restore must never be blindly resubmitted"
    );
    assert_eq!(
        handle.borrow().poll_calls,
        1,
        "reconciliation must query getTransaction for the restore"
    );
    assert_eq!(handle.borrow().submit_calls, 1);
}

#[test]
fn no_restore_footprint_skips_restoration_entirely() {
    // When simulation reports no archived entries, the client must go straight
    // to the real transaction and never touch the restore path.
    let node = Shared::new(
        ScriptedProvider::new("solo")
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 30 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let footprint = RestoreFootprint::empty();
    let result = client.submit_with_restore(
        &SignedTx::new("tx-no-archive", ""),
        &footprint,
        &mut log,
    );

    assert_eq!(
        result,
        RecoveryResult::Accepted {
            ledger: 30,
            provider: "solo".to_string(),
            submit_attempts: 1,
        }
    );
    assert_eq!(
        handle.borrow().restore_calls,
        0,
        "an empty footprint must not produce a restore transaction"
    );
    assert_eq!(handle.borrow().submit_calls, 1);
}
