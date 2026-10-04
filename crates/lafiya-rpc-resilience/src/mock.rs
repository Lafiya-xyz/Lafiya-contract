//! A scriptable fake [`RpcProvider`] for failure-injection tests and demos.
//!
//! Each call to `submit`/`get_transaction` pops the next scripted outcome
//! off a queue, so a test can lay out an exact failure sequence (e.g.
//! "timeout, then rate-limited, then accepted") and assert both the final
//! [`RecoveryResult`](crate::RecoveryResult) and the number of calls made to
//! each provider (to prove a duplicate submission never happened).

use crate::{RpcError, RpcProvider, SubmitOutcome, TxState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub struct ScriptedProvider {
    label: String,
    submit_script: VecDeque<SubmitOutcome>,
    poll_script: VecDeque<Result<TxState, RpcError>>,
    ledger_script: VecDeque<u32>,
    sequence_script: VecDeque<i64>,
    consumed_by: Option<String>,
    pub submit_calls: u32,
    pub poll_calls: u32,
}

impl ScriptedProvider {
    pub fn new(label: &str) -> Self {
        ScriptedProvider {
            label: label.to_string(),
            submit_script: VecDeque::new(),
            poll_script: VecDeque::new(),
            ledger_script: VecDeque::new(),
            sequence_script: VecDeque::new(),
            consumed_by: None,
            submit_calls: 0,
            poll_calls: 0,
        }
    }

    /// Queue the next `submit` call's outcome.
    pub fn then_submit(mut self, outcome: SubmitOutcome) -> Self {
        self.submit_script.push_back(outcome);
        self
    }

    /// Queue the next `get_transaction` call's result.
    pub fn then_poll(mut self, result: Result<TxState, RpcError>) -> Self {
        self.poll_script.push_back(result);
        self
    }

    /// Queue the next `latest_ledger` answer. The last queued value is
    /// sticky, so a test only scripts the ledgers where something changes.
    /// With nothing queued the provider does not report a ledger.
    pub fn then_latest_ledger(mut self, ledger: u32) -> Self {
        self.ledger_script.push_back(ledger);
        self
    }

    /// Queue the next `account_sequence` answer (sticky, like ledgers).
    pub fn then_account_sequence(mut self, sequence: i64) -> Self {
        self.sequence_script.push_back(sequence);
        self
    }

    /// Hash reported by `find_transaction_by_sequence`.
    pub fn with_consumed_by(mut self, tx_hash: &str) -> Self {
        self.consumed_by = Some(tx_hash.to_string());
        self
    }
}

fn next_sticky<T: Copy>(script: &mut VecDeque<T>) -> Option<T> {
    if script.len() > 1 {
        script.pop_front()
    } else {
        script.front().copied()
    }
}

impl RpcProvider for ScriptedProvider {
    fn name(&self) -> &str {
        &self.label
    }

    /// Falls back to `ProviderUnavailable` once the script runs out, rather
    /// than panicking -- a test that under-scripts a provider gets a clear,
    /// on-brand failure instead of an unrelated `unwrap` panic.
    fn submit(&mut self, _tx_hash: &str) -> SubmitOutcome {
        self.submit_calls += 1;
        self.submit_script
            .pop_front()
            .unwrap_or(SubmitOutcome::Definite(RpcError::ProviderUnavailable))
    }

    fn get_transaction(&mut self, _tx_hash: &str) -> Result<TxState, RpcError> {
        self.poll_calls += 1;
        self.poll_script.pop_front().unwrap_or(Ok(TxState::Unknown))
    }

    fn latest_ledger(&mut self) -> Result<u32, RpcError> {
        next_sticky(&mut self.ledger_script).ok_or(RpcError::ProviderUnavailable)
    }

    fn account_sequence(&mut self, _account: &str) -> Result<i64, RpcError> {
        next_sticky(&mut self.sequence_script).ok_or(RpcError::ProviderUnavailable)
    }

    fn find_transaction_by_sequence(
        &mut self,
        _account: &str,
        _sequence: i64,
    ) -> Result<Option<String>, RpcError> {
        Ok(self.consumed_by.clone())
    }
}

/// Wraps a provider in an `Rc<RefCell<_>>` so a caller can hold onto a
/// [`handle`](Shared::handle) for inspection (e.g. asserting `submit_calls`)
/// after moving the boxed provider into a [`crate::FailoverClient`].
pub struct Shared<P> {
    label: String,
    inner: Rc<RefCell<P>>,
}

impl<P: RpcProvider> Shared<P> {
    pub fn new(provider: P) -> Self {
        let label = provider.name().to_string();
        Shared {
            label,
            inner: Rc::new(RefCell::new(provider)),
        }
    }

    pub fn handle(&self) -> Rc<RefCell<P>> {
        self.inner.clone()
    }
}

impl<P: RpcProvider> RpcProvider for Shared<P> {
    fn name(&self) -> &str {
        &self.label
    }

    fn submit(&mut self, tx_hash: &str) -> SubmitOutcome {
        self.inner.borrow_mut().submit(tx_hash)
    }

    fn get_transaction(&mut self, tx_hash: &str) -> Result<TxState, RpcError> {
        self.inner.borrow_mut().get_transaction(tx_hash)
    }

    fn latest_ledger(&mut self) -> Result<u32, RpcError> {
        self.inner.borrow_mut().latest_ledger()
    }

    fn account_sequence(&mut self, account: &str) -> Result<i64, RpcError> {
        self.inner.borrow_mut().account_sequence(account)
    }

    fn find_transaction_by_sequence(
        &mut self,
        account: &str,
        sequence: i64,
    ) -> Result<Option<String>, RpcError> {
        self.inner
            .borrow_mut()
            .find_transaction_by_sequence(account, sequence)
    }
}
