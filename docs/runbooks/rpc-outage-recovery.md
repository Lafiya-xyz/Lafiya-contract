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

## 0. What `lafiya-cli` already does for you

`lafiya-cli attester add` / `attester remove` do **not** hand submission to a single
`stellar contract invoke` call any more. The CLI:

1. builds and simulates the transaction with `stellar contract invoke --build-only` and
   `stellar tx simulate`, trying each endpoint in `rpc_urls` in order (these steps are
   read-only);
2. signs it locally (`stellar tx sign`) and computes its hash (`stellar tx hash`);
3. submits it through `FailoverClient::submit_with_recovery` with the production
   `HttpRpcProvider`, across every endpoint in `rpc_urls` (or just `rpc_url` when the list
   is empty):
   - connection refused, DNS failure, TLS failure, HTTP 429/503, or `TRY_AGAIN_LATER`:
     nothing was recorded, so it retries on the next endpoint after a jittered backoff;
   - timeout after the request was written, a reset mid-response, or another 5xx: the
     outcome is ambiguous, so it **polls `getTransaction` on every endpoint** instead of
     resubmitting;
   - `ERROR` from `sendTransaction`, or `FAILED` from `getTransaction`: final, never retried.

Pass `-v` to print the full recovery log (`[rpc] round 1: submit <hash> via <url>` …).
The command ends in one of three ways:

| CLI output | Meaning | What to do |
| --- | --- | --- |
| `Transaction <hash> succeeded in ledger <n> (via <url>)` | Landed. | Nothing. |
| `transaction <hash> was rejected (<code>); do not resubmit it` | Final on-chain verdict. | See §3, last row. |
| `outcome of transaction <hash> is unknown …` | Every endpoint exhausted without a verdict. | Keep the hash and go to [§3a](#3a-poll-the-transaction-hash), then [§5](#5-escalate). |

The rest of this runbook covers the Bash scripts, other commands, and the "unknown" case.

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

- For `lafiya-cli attester add`/`remove`: the hash is computed before anything is sent and
  printed in every final message (and in the `-v` log). If the CLI stopped before printing
  one, it failed while building, simulating, or signing, so nothing was submitted.
- For `attest` and the Bash `scripts/admin.sh`: these are single-invocation calls signed and
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
- `"NOT_FOUND"` **and** `result.latestLedger` is greater than the transaction's
  `maxLedger` (the CLI prints `valid through ledger N` when it submits) — the transaction
  has **expired** and can never be included. Nothing was applied. It is safe to re-run the
  original command, which builds a fresh transaction with new bounds. `lafiya-cli` already
  does this automatically, up to two times.
- `"NOT_FOUND"` and the source account's sequence number (`stellar keys` / `getLedgerEntries`)
  is at or past the transaction's sequence number — a **different** transaction consumed the
  sequence number, so this one can never be included. Check what that transaction did
  before re-running anything, since it may already have made the same change.
- `"NOT_FOUND"` otherwise — this provider has no record of the hash. This means either it never
  arrived, or it aged out of this provider's retention window. If you have more than one RPC
  URL available for this network, repeat the query against each before concluding "not
  found" — see [§4](#4-if-the-provider-itself-is-down).
- Anything else (`"PENDING"`, no `result` yet) — wait a few seconds (start at ~1s, double
  each retry, cap around 8s — same schedule as `backoff_schedule` in
  `crates/lafiya-rpc-resilience`) and query again. Stop after 5 rounds and escalate
  ([§5](#5-escalate)) if it's still unresolved.

## 4. If the provider itself is down

List fallback endpoints per network in `rpc_urls` (primary first; `rpc_url` is used when
the list is empty). `lafiya-cli` then fails over automatically (§0):

```toml
[testnet]
rpc_url = "https://soroban-testnet.stellar.org"
rpc_urls = ["https://soroban-testnet.stellar.org", "https://<second-provider>"]
```

For a one-off, don't edit the tracked file. Put the list in the gitignored
`config/networks.local.toml`, or point `rpc_url` at a working provider with
`LAFIYA_<NETWORK>_RPC_URL=...` or `--set rpc_url=...` (either one replaces the whole list;
see `config/README.md`). For the Bash scripts and manual checks:

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
   LAFIYA_TESTNET_RPC_URL=https://<working-provider> ./scripts/admin.sh --network testnet attester add G...
   ```
   (Do not commit a temporary provider override into the tracked config file.)

## 5. Escalate

The security watchdog raises `watchdog_failing` / `watchdog_lag` (critical) when it
cannot reach RPC or falls behind; during an outage expect those alerts and follow
[watchdog.md](watchdog.md#alert-types). Every admin command run with `lafiya-cli`
during the incident is recorded in `~/.lafiya/audit.jsonl` (`lafiya-cli audit show`),
including the transaction hash when the stellar CLI reported one.

If §3a's poll budget is exhausted and no provider will confirm either `SUCCESS`, `FAILED`,
or a consistent `NOT_FOUND`, stop retrying. Record: the transaction hash, the command that
was run, the network, and every provider URL queried with its response. This is exactly the
`RecoveryResult::ExhaustedNeedsOperator` case in `crates/lafiya-rpc-resilience` — the
automated model also gives up at this point rather than guessing, and hands the same
information back for a human to resolve (e.g. by inspecting a block explorer, or waiting out
a provider's indexing lag before trying again later).

## 6. Transaction stuck pending (fee escalation)

**Scope:** the transaction is not ambiguous -- §3a's `getTransaction` poll returns
`"PENDING"` consistently across providers -- it is simply not being included, usually because
its inclusion fee lost a surge-pricing auction. This is a different problem from an RPC
outage: retrying or polling harder does not help; the transaction needs a higher bid. See
issue #408 and `crates/lafiya-rpc-resilience/src/fees.rs` for the calculator this section
describes.

1. **Confirm it's a fee problem, not an outage.** If `getTransaction` returns `"PENDING"`
   consistently (not `NOT_FOUND` on some providers), and several ledgers have closed since
   submission, the transaction reached the network but isn't winning inclusion. Compare its
   inclusion fee against current network conditions:
   ```sh
   curl -s "$LAFIYA_RPC_URL" -H 'content-type: application/json' \
     -d '{"jsonrpc":"2.0","id":1,"method":"getFeeStats"}' | python3 -m json.tool
   ```
2. **Resubmit as a fee-bump transaction**, not a fresh transaction: wrap the *same* signed
   envelope in a `FeeBumpTransaction` with a higher inclusion fee (`fees::decide_fee_bump`
   picks the new bid -- p90 of current `getFeeStats` by default, strictly above the original
   bid). The inner transaction's hash is unchanged by a fee bump, so §3a's poll-by-hash
   instructions apply unmodified to the wrapped submission -- keep polling the *inner* hash.
3. **Respect `--max-fee`.** If the computed bump would exceed the operator-set budget
   ceiling, `fees::enforce_budget` refuses it (`FeeError::ExceedsBudget`). Do not override
   this by hand without deliberately raising `--max-fee` first -- it exists so an automated
   or scripted retry can't runaway-bid during a fee spike.
4. **Emergency commands (`pause`, `emergency-stop`, `revoke`) start at high priority** (p90)
   rather than going through this escalation path from p50, since they're the case where
   minutes of delay matter most.
5. If a fee-bumped resubmission also sits pending past the same threshold, repeat from step
   2 with fresh `getFeeStats` data -- congestion can worsen between attempts. If it's still
   unresolved after a few rounds, escalate per [§5](#5-escalate); note in the escalation that
   this was a fee-congestion case, not an ambiguous-outcome case, so the next operator
   doesn't re-walk §3a expecting a different answer.

## 7. Ledger reorg / provider divergence (indexer)

**Scope:** the `lafiya-indexer` ingest loop (`crates/lafiya-indexer/src/ingest.rs`) has
detected a ledger-hash discontinuity — the tip served by the current RPC provider does not
extend the chain the indexer already persisted. This happens during provider failover or a
provider restart that serves a divergent tip. It is *not* an RPC outage (§4) and *not* an
ambiguous submission (§3a): the transaction outcome is known, but the indexed state may
reflect events from a ledger that was later dropped.

**What the indexer does automatically.** On every page it records the ledger hash and
verifies continuity against the last ingested ledger. On mismatch it rolls back the derived
tables to the common ancestor inside a single transaction and re-ingests from there. The
rollback is atomic and idempotent — re-running yields the same state — so in the common case
no operator action is needed. Watch for:

- `reorg_detected_total` incrementing (metric exposed by the indexer);
- the last-rollback depth metric rising (how many ledgers were rolled back).

A single small reorg (depth of a few ledgers) that resolves on its own is expected during
failover. Continue below only if the metric keeps climbing, the depth is large, or the
indexer cannot find a common ancestor.

1. **Confirm it is a reorg, not a stuck loop.** Check `reorg_detected_total` and the
   last-rollback depth. If the depth keeps growing or the counter increments on every page,
   the two providers likely disagree on history rather than one having a transient
   divergent tip.
2. **Pin the indexer to a single provider.** Mixed providers are the usual cause. Point the
   indexer at one known-good RPC URL (the same override mechanism as §4:
   `LAFIYA_<NETWORK>_RPC_URL=...` or `--set rpc_url=...`) and let it re-ingest from the
   common ancestor. Do not commit a temporary override into the tracked config.
3. **Verify the common ancestor.** If the indexer reports it cannot find a common ancestor
   within its retention window, stop it before it ingests further. Compare the ledger hash
   at the last known-good ledger (the one before the first `reorg_detected_total` increment)
   across every configured provider using `getLedger`/`getLedgers`. A provider whose hash
   for that ledger differs from the majority is the divergent one — remove it from
   `rpc_urls` until it is resynced.
4. **Re-ingest deterministically.** Once pinned to a consistent provider, restart the
   indexer. Because rollback is idempotent, re-running from the checkpoint yields the same
   state; no manual table surgery is required. If you must force a deeper rollback than the
   indexer chose, do it only by moving the checkpoint back to a ledger whose hash you have
   verified across providers — never by deleting rows by hand.
5. **Tell API consumers about the confidence boundary.** API responses expose
   `finalized_ledger`. After a reorg, `finalized_ledger` moves back to the new common
   ancestor and only advances again as fresh ledgers are ingested. Consumers that need
   finality should treat events at or below `finalized_ledger` as settled and anything above
   it as provisional. If a consumer cached data derived from ledgers above the new
   `finalized_ledger`, it must re-fetch.
6. **Escalate** per [§5](#5-escalate) if the depth is large, providers cannot be reconciled,
   or the indexer cannot make progress. Record: the network, every provider URL and the
   ledger hash each reports for the disputed ledger(s), the `reorg_detected_total` value and
   last-rollback depth at the time you intervened, and the checkpoint ledger you rolled back
   to. Note explicitly that this was a reorg/divergence case so the next operator does not
   re-walk §3a or §4 expecting a different answer.
