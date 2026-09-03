//! Runnable failure-injection demo for operators and reviewers.
//!
//! ```sh
//! cargo run -p lafiya-rpc-resilience --example failure_injection_demo
//! ```
//!
//! Prints the recovery-log trace for each scenario in
//! `docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md`'s
//! evaluation, so the timeout / ambiguous-submit / failover behavior can be
//! inspected without standing up a Soroban RPC endpoint. The same scenarios
//! are asserted precisely (final result + call counts) in
//! `tests/failure_injection.rs`; this binary is the human-readable version
//! of the same failure injection for an operator running the recovery
//! runbook by hand.

use lafiya_rpc_resilience::mock::ScriptedProvider;
use lafiya_rpc_resilience::{
    FailoverClient, RecoveryLog, RetryPolicy, RpcError, RpcProvider, SubmitOutcome, TxState,
};
use std::time::Duration;

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_submit_rounds: 4,
        max_poll_rounds: 4,
        base_backoff: Duration::from_millis(250),
        max_backoff: Duration::from_secs(4),
    }
}

fn run(title: &str, tx_hash: &str, providers: Vec<Box<dyn RpcProvider>>) {
    println!("=== {title} ===");
    let mut client = FailoverClient::new(providers, policy());
    let mut log = RecoveryLog::new();
    let result = client.submit_with_recovery(tx_hash, &mut log);
    for line in log.lines() {
        println!("  {line}");
    }
    println!("  -> {result:?}\n");
}

fn main() {
    run(
        "Scenario 1: timeout before the request reached the provider (safe retry)",
        "tx-scenario-1",
        vec![Box::new(
            ScriptedProvider::new("testnet-primary")
                .then_submit(SubmitOutcome::Definite(RpcError::Timeout))
                .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1234 })),
        )],
    );

    run(
        "Scenario 2: ambiguous timeout after send, chain actually accepted it",
        "tx-scenario-2",
        vec![Box::new(
            ScriptedProvider::new("testnet-primary")
                .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
                .then_poll(Ok(TxState::Pending))
                .then_poll(Ok(TxState::Accepted { ledger: 1235 })),
        )],
    );

    run(
        "Scenario 3: primary provider hard down, secondary provider takes over",
        "tx-scenario-3",
        vec![
            Box::new(
                ScriptedProvider::new("provider-a")
                    .then_submit(SubmitOutcome::Definite(RpcError::ProviderUnavailable)),
            ),
            Box::new(
                ScriptedProvider::new("provider-b")
                    .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1236 })),
            ),
        ],
    );

    run(
        "Scenario 4: rate limited, then succeeds after backoff",
        "tx-scenario-4",
        vec![Box::new(
            ScriptedProvider::new("testnet-primary")
                .then_submit(SubmitOutcome::Definite(RpcError::RateLimited {
                    retry_after: Some(Duration::from_secs(1)),
                }))
                .then_submit(SubmitOutcome::Ack(TxState::Accepted { ledger: 1237 })),
        )],
    );

    run(
        "Scenario 5: ambiguous submit, chain actually rejected it (do not retry same tx)",
        "tx-scenario-5",
        vec![Box::new(
            ScriptedProvider::new("testnet-primary")
                .then_submit(SubmitOutcome::Ambiguous(RpcError::Timeout))
                .then_poll(Ok(TxState::Rejected {
                    reason: "source account sequence number already consumed".to_string(),
                })),
        )],
    );
}
