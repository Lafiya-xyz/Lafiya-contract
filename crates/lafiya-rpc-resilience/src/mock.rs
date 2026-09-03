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
    pub submit_calls: u32,
    pub poll_calls: u32,
}

impl ScriptedProvider {
    pub fn new(label: &str) -> Self {
        ScriptedProvider {
            label: label.to_string(),
            submit_script: VecDeque::new(),
            poll_script: VecDeque::new(),
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
}
