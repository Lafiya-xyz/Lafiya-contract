# Chaos scenarios

A fault-injecting JSON-RPC proxy (`chaos.py`) that sits between tooling and a
Soroban RPC and breaks one thing per scenario. `scenarios.toml` is the DSL:

```toml
[[scenario]]
name = "stale_tip"                 # shown in results
fault = "stale_tip"                # key of FAULTS in chaos.py (omit for pass-through)
options = { lag = 100 }            # passed to the fault
command = ["python3", "sandbox/chaos/probe.py", "{rpc_url}"]  # {rpc_url} = proxy
expect_exit = 2                    # assertions on the tooling under test
expect_output = ["stale tip"]
```

Run: `python3 sandbox/chaos/chaos.py run sandbox/chaos/scenarios.toml`
(add `--upstream <rpc url>` to proxy a real sandbox instead of the built-in stub).

## Faults

`latency`, `truncated_body`, `malformed_json`, `http_500`, `rate_limited`,
`connection_drop`, `empty_result`, `stale_tip`, `ledger_gap`, `clock_skew`,
`lying_rpc` (11).

## Add a fault

1. Write `def my_fault(req, body, opts) -> (status, body_bytes)` in `chaos.py`
   (raise `ConnectionAbortedError` to drop the socket).
2. Add it to the `FAULTS` tuple.
3. Add a `[[scenario]]` to `scenarios.toml` with the expected behavior.

To exercise the real indexer or watchdog, point `command` at the binary with
`{rpc_url}` as its RPC URL (for example `lafiya-watchdog --config` pointing at a
generated config) and assert on exit code and output. The nightly workflow
(`.github/workflows/chaos.yml`) runs every scenario; a failing scenario is a
finding, so file or fix it.
