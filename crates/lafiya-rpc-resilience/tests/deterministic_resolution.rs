//! Deterministic resolution of ledger-bounded transactions (issue #407).
//!
//! These drive [`FailoverClient::submit_with_bounds`] through a dropped
//! submission (the provider acknowledges nothing and never forwards the
//! transaction) and assert it resolves to `Expired` only once `maxLedger`
//! has closed, stops polling as soon as the outcome is certain, and never
//! resubmits the same envelope.

use std::cell::RefCell;
use std::rc::Rc;

use lafiya_rpc_resilience::mock::{ScriptedProvider, Shared};
use lafiya_rpc_resilience::{
    classify, FailoverClient, HealthConfig, HealthTracker, RecoveryLog, RecoveryResult,
    RetryClass, RetryPolicy, RpcError, RpcProvider, SignedTx, SubmitOutcome, TxBounds, TxState,
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

    let result = client.submit_with_bounds(&SignedTx::new("tx-dropped", ""), &bounds(), &mut log);

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

    let first = client.submit_with_bounds(
        &SignedTx::new("tx-first", ""),
        &bounds(),
        &mut RecoveryLog::new(),
    );
    assert!(matches!(first, RecoveryResult::Expired { .. }));

    let rebuilt = TxBounds {
        max_ledger: 1_121,
        ..bounds()
    };
    let second = client.submit_with_bounds(
        &SignedTx::new("tx-rebuilt", ""),
        &rebuilt,
        &mut RecoveryLog::new(),
    );
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

    let result = client.submit_with_bounds(
        &SignedTx::new("tx-late", ""),
        &bounds(),
        &mut RecoveryLog::new(),
    );
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

    let result = client.submit_with_bounds(
        &SignedTx::new("tx-mine", ""),
        &bounds(),
        &mut RecoveryLog::new(),
    );
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

    let result = client.submit_with_bounds(
        &SignedTx::new("tx-blind", ""),
        &bounds(),
        &mut RecoveryLog::new(),
    );
    assert_eq!(
        result,
        RecoveryResult::ExhaustedNeedsOperator {
            last_known: TxState::Unknown
        }
    );
    assert_eq!(handle.borrow().poll_calls, 2);
}

// ---------------------------------------------------------------------------
// Latency-aware scoring, hedging and half-open circuit breaking (issue #540).
// ---------------------------------------------------------------------------

fn health_config() -> HealthConfig {
    HealthConfig {
        latency_percentile: 0.5,
        hedge_after: std::time::Duration::from_millis(5),
        error_penalty: 4.0,
        staleness_penalty: 1.0,
        open_threshold: 3,
        half_open_after: std::time::Duration::from_millis(1),
        half_open_jitter: std::time::Duration::from_millis(1),
        ..HealthConfig::default()
    }
}

/// A provider that reports a fixed latency on every call, so the simulation
/// is fully deterministic (no wall-clock sleeps).
fn latency_provider(name: &'static str, latency: std::time::Duration) -> Box<dyn RpcProvider> {
    Box::new(ScriptedProvider::new(name).with_latency(latency))
}

#[test]
fn latency_aware_scoring_recovers_faster_than_static_ordering() {
    // Provider A is alive but slow; provider B is fast. Static ordering keeps
    // picking A and pays its latency every round. Latency-aware scoring learns
    // to prefer B and recovers in fewer simulated milliseconds.
    let build = || -> (Vec<Box<dyn RpcProvider>>, Rc<RefCell<HealthTracker>>) {
        let tracker = Rc::new(RefCell::new(HealthTracker::new(health_config())));
        let providers: Vec<Box<dyn RpcProvider>> = vec![
            latency_provider("slow", std::time::Duration::from_millis(50)),
            latency_provider("fast", std::time::Duration::from_millis(1)),
        ];
        (providers, tracker)
    };

    let (providers, tracker) = build();
    let mut baseline = FailoverClient::new(providers, policy());
    let baseline_elapsed = baseline.simulated_elapsed();

    let (providers, tracker) = build();
    let mut scored = FailoverClient::with_health(providers, policy(), tracker.clone());
    let scored_elapsed = scored.simulated_elapsed();

    assert!(
        scored_elapsed < baseline_elapsed,
        "latency-aware scoring ({scored_elapsed:?}) should beat static ordering ({baseline_elapsed:?})"
    );
    assert!(tracker.borrow().score("fast") > tracker.borrow().score("slow"));
}

#[test]
fn hedged_reads_use_faster_replica_but_submissions_stay_single_flight() {
    let slow = Shared::new(
        ScriptedProvider::new("slow")
            .with_latency(std::time::Duration::from_millis(50))
            .then_poll(Ok(TxState::Accepted { ledger: 1_050 })),
    );
    let fast = Shared::new(
        ScriptedProvider::new("fast")
            .with_latency(std::time::Duration::from_millis(1))
            .then_poll(Ok(TxState::Accepted { ledger: 1_050 })),
    );
    let slow_handle = slow.handle();
    let fast_handle = fast.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(slow), Box::new(fast)];
    let tracker = Rc::new(RefCell::new(HealthTracker::new(health_config())));
    let mut client = FailoverClient::with_health(providers, policy(), tracker);

    let state = client
        .poll_hedged(&SignedTx::new("tx-hedge", ""))
        .expect("hedged read resolves");
    assert_eq!(state, TxState::Accepted { ledger: 1_050 });
    assert_eq!(
        fast_handle.borrow().poll_calls,
        1,
        "the fast replica answers the hedge"
    );
    assert_eq!(
        slow_handle.borrow().poll_calls,
        1,
        "the slow replica is still consulted once"
    );
    assert_eq!(
        slow_handle.borrow().submit_calls + fast_handle.borrow().submit_calls,
        0,
        "hedging never submits"
    );
}

#[test]
fn half_open_breaker_recovers_with_jittered_probe() {
    let node = Shared::new(
        ScriptedProvider::new("flaky")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1_070 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let tracker = Rc::new(RefCell::new(HealthTracker::new(health_config())));
    let mut client = FailoverClient::with_health(providers, policy(), tracker.clone());

    // Three failures trip the breaker open.
    for _ in 0..3 {
        let _ = client.submit_with_bounds(
            &SignedTx::new("tx-breaker", ""),
            &bounds(),
            &mut RecoveryLog::new(),
        );
    }
    assert!(tracker.borrow().is_open("flaky"));

    // After the half-open delay a single jittered probe is admitted and, on
    // success, the breaker closes again.
    client.advance_simulated(std::time::Duration::from_millis(2));
    let result = client.submit_with_bounds(
        &SignedTx::new("tx-breaker", ""),
        &bounds(),
        &mut RecoveryLog::new(),
    );
    assert!(matches!(
        result,
        RecoveryResult::Accepted { ledger: 1_070, .. }
    ));
    assert!(!tracker.borrow().is_open("flaky"));
    assert_eq!(handle.borrow().submit_calls, 4);
}

#[test]
fn submission_is_single_flight_and_idempotent_by_hash() {
    let node = Shared::new(
        ScriptedProvider::new("node")
            .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
            .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1_050 })),
    );
    let handle = node.handle();
    let providers: Vec<Box<dyn RpcProvider>> = vec![Box::new(node)];
    let tracker = Rc::new(RefCell::new(HealthTracker::new(health_config())));
    let mut client = FailoverClient::with_health(providers, policy(), tracker);

    let tx = SignedTx::new("tx-idem", "");
    let first = client.submit_with_bounds(&tx, &bounds(), &mut RecoveryLog::new());
    let second = client.submit_with_bounds(&tx, &bounds(), &mut RecoveryLog::new());

    assert!(matches!(first, RecoveryResult::Accepted { .. }));
    assert_eq!(
        first, second,
        "resubmitting the same hash is idempotent"
    );
    assert_eq!(
        handle.borrow().submit_calls,
        1,
        "the envelope is submitted exactly once (single-flight)"
    );
}
