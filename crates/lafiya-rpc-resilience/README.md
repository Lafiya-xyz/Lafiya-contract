# lafiya-rpc-resilience

Prototype produced for the RPC provider failover and transaction-recovery
spike ([issue #132](https://github.com/Lafiya-xyz/Lafiya-contract/issues/132)).
The decision record is
[`docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md`](../../docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md);
this crate is that ADR's failure-injection prototype and transaction-state
model, expressed as runnable code instead of prose.

It is **not** a Soroban RPC client. It has no HTTP dependency and does not
know anything about Stellar transaction envelopes. It is the reusable piece
underneath a real client: given a submit attempt's outcome, classify whether
retrying is safe, and if not, poll a transaction hash to a final verdict
across an ordered list of providers before allowing a retry.

## Why a separate crate

The bug this models is provider-agnostic and does not need XDR, signing, or
a live network to demonstrate: "the HTTP call failed" collapses two very
different situations -- *nothing was sent* (safe to retry) and *something
may have been sent and we don't know what happened to it* (unsafe to retry
until polled) -- into the same generic error. [`SubmitOutcome`] keeps those
cases distinct at the type level so a caller cannot accidentally treat one
as the other.

## Layout

- `src/lib.rs` -- `TxState`, `RpcError`, `SubmitOutcome`, `RetryClass` /
  [`classify`], `backoff_schedule`, and `FailoverClient::submit_with_recovery`
  (the round-robin-submit / poll-before-retry state machine).
- `src/mock.rs` -- `ScriptedProvider`, a fake `RpcProvider` whose
  `submit`/`get_transaction` responses are scripted in advance, plus
  `Shared`, a small `Rc<RefCell<_>>` wrapper so a test can keep a handle to
  assert call counts after handing the provider to a `FailoverClient`.
- `tests/failure_injection.rs` -- the scenarios from the ADR's evaluation
  (timeout before send, ambiguous timeout after send resolving to accepted
  or rejected, rate limiting, primary-down failover, and both retry-budget
  and poll-budget exhaustion), each asserting the final result *and* the
  exact number of calls made to each provider.
- `examples/failure_injection_demo.rs` -- the same scenarios, printed as a
  human-readable recovery-log trace. Run with:

  ```sh
  cargo run -p lafiya-rpc-resilience --example failure_injection_demo
  ```

## Running the tests

```sh
cargo test -p lafiya-rpc-resilience
```

## Scope and limitations

- No jitter in `backoff_schedule`; see its doc comment. A production client
  should add jitter before pointing many concurrent callers at one provider.
- No async runtime. The admin CLI and scripts in this repository are
  synchronous, low-frequency callers (an operator running one command), so
  this prototype models the same shape. An indexer or high-throughput
  service would need a different concurrency model.
- No live-provider comparison harness. The provider comparison matrix in
  the ADR is evaluated qualitatively (documented behavior, published SLAs,
  and this repository's own experience), not by load-testing real
  endpoints -- that is listed as follow-up work.
