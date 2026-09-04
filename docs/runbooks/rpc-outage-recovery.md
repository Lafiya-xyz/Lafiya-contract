# Runbook: Recovering from an RPC Outage or Ambiguous Submission

**Audience:** anyone running `scripts/admin.sh`, `scripts/deploy.sh`, `scripts/upgrade.sh`,
or `cargo run -p lafiya-cli` and hitting a timeout, connection error, or rate limit.

**Scope:** what to do *after* a command against `config/networks.toml`'s configured RPC
endpoint fails or hangs, before deciding whether to re-run it. Not a description of how to
configure networks (see `config/README.md`) or how to upgrade a contract (see
[`contract-upgrade.md`](contract-upgrade.md)).

**Background:** see
[ADR-0011](../adr/0011-rpc-provider-failover-and-transaction-recovery.md) for the full
retry-classification model and the failure-injection prototype this runbook mirrors
(`crates/lafiya-rpc-resilience`). The one fact this whole runbook hangs off:

> A network failure does not tell you whether your transaction happened. Only the ledger
> does. Never re-run a state-changing command based on a timeout alone — check the ledger
> first.

---

## 1. Is this a read or a write?

- **Read-only** (`config show`, `attester is`, `attestation get`, `stellar tx simulate`):
  safe to just retry. Nothing was ever recorded on-chain by a read. Go to
  [§4](#4-if-the-provider-itself-is-down) if it keeps failing.
- **State-changing** (`attester add`/`remove`, `attest`, `deploy`, `upgrade`): do **not**
  re-run the command yet. Continue to [§2](#2-find-the-transaction-hash).

## 2. Find the transaction hash

`stellar` CLI invocations print the transaction hash before or as part of failing —
check the command's stderr/stdout output first. If the process was killed before printing
one (or you don't have the terminal output anymore):

- For `attester add`/`remove` and `attest`: these are single-invocation calls signed and
  submitted in one step; if no hash was printed, the request most likely never reached the
  network (Definite failure — see §3, retry directly). This is the one case where "no hash"
  is itself informative.
- For `deploy`/`upgrade`: check `scripts/deploy.sh`'s or `scripts/upgrade.sh`'s printed
  progress log — both echo each `stellar contract ...` invocation's arguments before running
  it, including any hash it produced.

If you have a hash, keep it — every remaining step in this runbook is keyed on it.

## 3. Classify the failure

| What you saw | Class (see ADR-0011) | What to do |
| --- | --- | --- |
| Connection refused, DNS failure, or an error *before* any hash was printed | **Safe to retry** | Re-run the exact same command. |
| HTTP 429 / "rate limited" | **Safe to retry, after waiting** | Wait (start at a few seconds, double if it recurs), then re-run the exact same command. |
| Timeout, connection reset, or the process was killed *after* a hash was printed | **Must poll first** | Go to [§3a](#3a-poll-the-transaction-hash). Do **not** re-run the command yet. |
| The CLI printed an explicit on-chain rejection (a `Error(...)` result, not a network error) | **Do not retry this transaction** | The chain has a final verdict. If you still want the effect, build and sign a fresh transaction — do not attempt to resubmit anything referencing the old hash. |

### 3a. Poll the transaction hash

Query the configured RPC endpoint's `getTransaction` JSON-RPC method directly — this works
regardless of which `stellar` CLI subcommands are installed:

```sh
source ./scripts/lib/config.sh
load_network_config "testnet"   # or your network

curl -s "$LAFIYA_RPC_URL" \
  -H 'content-type: application/json' \
  -d '{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": { "hash": "<the transaction hash from step 2>" }
      }' | python3 -m json.tool
```

Read `result.status`:

- `"SUCCESS"` — the transaction landed. **Stop here — do not re-run the original command.**
  Verify the effect instead (e.g. `./scripts/admin.sh --network testnet attester is <addr>`).
- `"FAILED"` — the transaction was included but failed on-chain. Read `result.resultXdr` (or
  re-run with `--network` pointed at a block explorer if you have one configured) for why.
  Treat as **do not retry this transaction** (§3, last row).
- `"NOT_FOUND"` — this provider has no record of the hash. This means either it never
  arrived, or it aged out of this provider's retention window. If you have more than one RPC
  URL available for this network, repeat the query against each before concluding "not
  found" — see [§4](#4-if-the-provider-itself-is-down).
- Anything else (`"PENDING"`, no `result` yet) — wait a few seconds (start at ~1s, double
  each retry, cap around 8s — same schedule as `backoff_schedule` in
  `crates/lafiya-rpc-resilience`) and query again. Stop after 5 rounds and escalate
  ([§5](#5-escalate)) if it's still unresolved.

## 4. If the provider itself is down

`config/networks.toml` currently defines one `rpc_url` per network (ADR-0011 proposes
extending this to a list; until that lands, do this manually):

1. Get a second known-good RPC URL for the same network (a different SDF endpoint, a
   self-hosted node, or a third-party provider — see ADR-0011's provider comparison matrix
   for the trade-offs of each).
2. Re-run §3a's `getTransaction` query against that second URL before doing anything else —
   a provider-specific outage does not mean the transaction failed, only that *this*
   provider can't tell you.
3. Only once you've confirmed via §3a (on any reachable provider) that the transaction is
   `NOT_FOUND` everywhere, treat it as safe to retry and re-run the original command,
   pointed at the working provider:
   ```sh
   ./scripts/admin.sh --network testnet --config /path/to/alt-networks.toml attester add G...
   ```
   (Use a scratch copy of `networks.toml` with `rpc_url` swapped — do not commit a
   temporary provider override into the tracked config file.)

## 5. Escalate

If §3a's poll budget is exhausted and no provider will confirm either `SUCCESS`, `FAILED`,
or a consistent `NOT_FOUND`, stop retrying. Record: the transaction hash, the command that
was run, the network, and every provider URL queried with its response. This is exactly the
`RecoveryResult::ExhaustedNeedsOperator` case in `crates/lafiya-rpc-resilience` — the
automated model also gives up at this point rather than guessing, and hands the same
information back for a human to resolve (e.g. by inspecting a block explorer, or waiting out
a provider's indexing lag before trying again later).
