//! Deterministic resolution of ledger-bounded transactions (issue #407).
//!
//! These drive [`FailoverClient::submit_with_bounds`] through a dropped
//! submission (the provider acknowledges nothing and never forwards the
//! transaction) and assert it resolves to `Expired` only once `maxLedger`
//! has closed, stops polling as soon as the outcome is certain, and never
//! resubmits the same envelope.

use lafiya_rpc_resilience::mock::{ScriptedProvider, Shared};
use lafiya_rpc_resilience::{
    classify, FailoverClient, RecoveryLog, RecoveryResult, RetryClass, RetryPolicy, RpcError,
    RpcProvider, SubmitOutcome, TxBounds, TxState,
};

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_submit_rounds: 3,
        max_poll_rounds: 2,
        base_backoff: std::time::Duration::from_millis(1),
        max_backoff: std::time::Duration::from_millis(2),
    }
}

fn bounds() -> TxBounds {
    TxBounds {
        max_ledger: 1_060,
        source_account: "GSOURCE".to_string(),
        sequence: 7,
    }
}

#[test]
fn dropped_submission_resolves_to_expired_after_max_ledger() {
    // The ledger advances past the bound over more rounds than
    // `max_poll_rounds`: the bounded client must keep polling until it is
    // certain rather than escalating after a fixed number of rounds.
    let node = Shared::new(
        ScriptedProvider::new("dropping-node")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_latest_ledger(1_000)
            .then_latest_ledger(1_030)
            .then_latest_ledger(1_060)
            .then_latest_ledger(1_061)
            .then_account_sequence(6),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();

    let result = client.submit_with_bounds("tx-dropped", &bounds(), &mut log);

    assert_eq!(
        result,
        RecoveryResult::Expired {
            max_ledger: 1_060,
            latest_ledger: 1_061
        }
    );
    assert_eq!(handle.borrow().submit_calls, 1, "never resubmitted");
    assert_eq!(
        handle.borrow().poll_calls,
        4,
        "polling stops the moment expiry is certain"
    );
    assert_eq!(
        classify(&SubmitOutcome::Ack(TxState::Expired {
            max_ledger: 1_060,
            latest_ledger: 1_061
        })),
        RetryClass::SafeToRebuild
    );
}

#[test]
fn expired_transaction_is_safely_rebuilt_and_lands() {
    let node = Shared::new(
        ScriptedProvider::new("node")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_latest_ledger(1_061)
            .then_account_sequence(6)
            // The rebuilt envelope (new hash, new bounds) is accepted.
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1_062 })),
    );
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());

    let first = client.submit_with_bounds("tx-first", &bounds(), &mut RecoveryLog::new());
    assert!(matches!(first, RecoveryResult::Expired { .. }));

    let rebuilt = TxBounds {
        max_ledger: 1_121,
        ..bounds()
    };
    let second = client.submit_with_bounds("tx-rebuilt", &rebuilt, &mut RecoveryLog::new());
    assert!(matches!(
        second,
        RecoveryResult::Accepted { ledger: 1_062, .. }
    ));
}

#[test]
fn inclusion_before_max_ledger_wins_over_expiry() {
    let node = ScriptedProvider::new("node")
        .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
        .then_poll(Ok(TxState::Unknown))
        .then_poll(Ok(TxState::Accepted { ledger: 1_050 }))
        .then_latest_ledger(1_040)
        .then_latest_ledger(1_070)
        .then_account_sequence(6);
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());

    let result = client.submit_with_bounds("tx-late", &bounds(), &mut RecoveryLog::new());
    assert!(matches!(
        result,
        RecoveryResult::Accepted { ledger: 1_050, .. }
    ));
}

#[test]
fn advanced_account_sequence_resolves_as_consumed() {
    let node = ScriptedProvider::new("node")
        .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
        .then_latest_ledger(1_010)
        .then_account_sequence(7)
        .with_consumed_by("tx-conflicting");
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());

    let result = client.submit_with_bounds("tx-mine", &bounds(), &mut RecoveryLog::new());
    assert_eq!(
        result,
        RecoveryResult::SequenceConsumed {
            consumed_by: Some("tx-conflicting".to_string())
        }
    );
}

#[test]
fn no_ledger_information_escalates_instead_of_polling_forever() {
    let node = Shared::new(
        ScriptedProvider::new("blind").then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout)),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let mut client = FailoverClient::new(providers, policy());

    let result = client.submit_with_bounds("tx-blind", &bounds(), &mut RecoveryLog::new());
    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::Unknown
        }
    );
    assert_eq!(handle.borrow().poll_calls, 2);
}
