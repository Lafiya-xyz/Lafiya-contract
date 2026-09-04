# ADR-0011: RPC provider failover and transaction-recovery strategy

- **Status:** Proposed
- **Date:** 2026-08-18
- **Deciders:** Lafiya contract maintainers

## Context

[Issue #132](https://github.com/Lafiya-xyz/Lafiya-contract/issues/132) is a spike asking how
Lafiya should recover from Soroban RPC outages, timeouts, rate limits, and ambiguous
transaction-submission results. Today `config/networks.toml` defines exactly one `rpc_url`
per network (see `config/README.md`), and both `scripts/admin.sh` / `scripts/deploy.sh`
(via `scripts/lib/config.sh`) and `crates/lafiya-cli` (`crates/lafiya-cli/src/main.rs`)
shell out to the `stellar` CLI against that single endpoint. There is no provider failover,
no persisted record of an in-flight transaction, and no distinction between "the RPC call
failed and nothing happened" and "the RPC call failed and we don't know what happened."

That distinction matters differently depending on which contract call is involved:

- `AttesterRegistry::add_attester` and `add_attester_with_info`
  (`contracts/attester-registry/src/lib.rs:249,283`) already check `already_present` before
  writing, and are a no-op if the attester is already allowlisted. `remove_attester`
  (`contracts/attester-registry/src/lib.rs:321`) is naturally idempotent the same way.
  Resubmitting a *new* transaction for one of these calls after an ambiguous failure is low
  risk: at worst it costs an extra fee and a redundant ledger entry, never a duplicated
  allowlist effect.
- `AttestationRegistry::attest` (`contracts/attestation-registry/src/lib.rs:294`) is **not**
  idempotent: every call increments `AttestationSequence(record_hash)` and appends a new
  `Attestation` entry (`contracts/attestation-registry/src/lib.rs:310-323`). Resubmitting a
  *new* attestation transaction after an ambiguous failure creates a second, distinct
  attestation record for the same `record_hash` — an operationally visible duplicate, not
  just a wasted fee.

So "is it safe to retry" cannot be answered once for "the CLI" — it depends on which call is
in flight and on what class of failure occurred. This ADR defines that classification and a
recovery model, and ships a dependency-free prototype (`crates/lafiya-rpc-resilience`) that
demonstrates it against injected failures rather than a live network.

## Decision

Adopt a **client-side failover list with poll-before-retry recovery**, implemented as the
`FailoverClient` state machine prototyped in
[`crates/lafiya-rpc-resilience`](../../crates/lafiya-rpc-resilience). No new hosted
infrastructure is introduced; `config/networks.toml` gains an ordered list of RPC endpoints
per network instead of one, and the CLI/scripts gain a transaction-hash-based recovery step
before any retry.

### Transaction-state model

Every submission is tracked through one of these states, queried via `getTransaction`
rather than assumed from the submit response (`TxState` in `src/lib.rs`):

`NotSubmitted → Submitted → Pending → Accepted{ledger} | Rejected{reason}`, with `Unknown`
as an explicit terminal-for-now state when a provider has no record of the hash (never seen
it, or it aged out of that provider's retention window).

### Retry classes

Every submit outcome is classified into exactly one `RetryClass` (`classify()` in
`src/lib.rs`), which is the safe/unsafe boundary the spike's acceptance criteria asks for:

| Outcome | Class | Why |
| --- | --- | --- |
| Pre-send / provider-rejected-before-processing failure (e.g. connection refused, malformed request) | **Safe to retry** | Nothing reached the network; no state changed. |
| Timeout or connection loss *after* the request may have been sent | **Must poll first** | Unknown whether the network has it. Resubmitting a *new* transaction here is the duplicate-submission risk the issue calls out; the recovery loop polls the transaction hash instead. |
| Chain returned a final `Rejected` verdict | **Do not retry this transaction** | Its sequence number is consumed either way; a genuinely new attempt needs a newly built and signed transaction, which is a decision for the caller, not this recovery loop. |
| Chain returned a final `Accepted` verdict | **No retry needed** | Done. |

The "must poll first" class is enforced structurally, not by convention: `FailoverClient`
never resubmits after an `Ambiguous` outcome or a queued (`Pending`/`Submitted`) one — it
always transitions to polling by hash. The failure-injection tests
(`tests/failure_injection.rs`) assert this directly by counting `submit_calls`, e.g.
`ambiguous_timeout_after_send_polls_instead_of_resubmitting` asserts exactly one submit call
occurred even though the transaction was ultimately confirmed two poll rounds later.

### Timeout and ambiguous-submit behavior, demonstrated

`crates/lafiya-rpc-resilience` reproduces the specific failure modes named in the issue
against a scripted fake provider (`src/mock.rs`), with no network or live RPC endpoint
required:

- **Timeout before send** (safe retry, same provider) —
  `timeout_before_send_is_safe_to_retry_and_succeeds`.
- **Ambiguous timeout after send, chain accepted it** (poll finds success, no duplicate
  submit) — `ambiguous_timeout_after_send_polls_instead_of_resubmitting`.
- **Ambiguous timeout after send, chain rejected it** (poll finds the final verdict, still
  no duplicate submit) — `ambiguous_timeout_after_send_polls_and_finds_rejection`.
- **Rate limiting** (definite failure, backs off, retries) —
  `rate_limit_backs_off_then_succeeds_on_the_same_provider`.
- **Primary provider hard down** (fails over to the next configured provider, each provider
  submitted to exactly once) —
  `primary_provider_down_fails_over_to_secondary_without_duplicate_submission`.
- **Retry budget exhausted** and **poll budget exhausted** (both escalate to the operator
  rather than looping forever or guessing) —
  `exhausting_submit_rounds_escalates_to_the_operator_as_not_submitted` and
  `exhausting_poll_rounds_escalates_to_the_operator_as_unknown`.

Run `cargo test -p lafiya-rpc-resilience` for the assertions, or
`cargo run -p lafiya-rpc-resilience --example failure_injection_demo` for a human-readable
recovery-log trace of the same scenarios — this is also the "recovery steps are reproducible
by an operator" check: the demo's printed trace is the shape of what a real client should
log, and matches the steps in the new runbook (see Follow-up).

### Retry policy

`RetryPolicy` bounds both the submit round-robin and the poll loop (`src/lib.rs`):
`max_submit_rounds` caps how many provider attempts a definite failure chain gets before
escalating as not-submitted; `max_poll_rounds` caps how many polling rounds an ambiguous
outcome gets before escalating as unknown. Both use `backoff_schedule` (exponential, capped,
no jitter — see its doc comment for why jitter is out of scope for this prototype) between
attempts. Defaults (`RetryPolicy::default()`) are conservative: 3 submit rounds, 5 poll
rounds, 250ms base backoff capped at 8s — tuned for a low-frequency, high-consequence admin
CLI, not a high-throughput service.

### Provider comparison matrix (at least two approaches evaluated)

| Approach | Description | Reliability | Latency | Cost | Operational complexity | Duplicate-submission risk |
| --- | --- | --- | --- | --- | --- | --- |
| **A. Client-side failover list** (adopted) | CLI/scripts hold an ordered list of RPC URLs per network and round-robin through them, polling by hash on ambiguity. | Good — survives any single provider's outage or rate limit. | Adds latency only on failure (extra round-trip per failover/poll). | None — no new hosted component. | Low — a config-file change and the recovery loop in this crate; no new service to run or monitor. | Low, if poll-before-retry is followed (this ADR's model). |
| **B. Dedicated RPC gateway/load balancer** | A reverse-proxy sidecar (self-hosted or off-the-shelf) fronts multiple providers behind one URL; callers are unaware of failover. | Good, and shared across every caller (CLI, `lafiya-web`, verifier clients) instead of re-implemented per client. | Similar to A once warmed; adds one more network hop always, not just on failure. | New always-on infrastructure to run, monitor, and pay for. | Higher — a service to deploy, secure, and keep patched. | Depends entirely on the gateway's own retry logic; without the same poll-before-retry discipline it can reintroduce this exact bug for every caller behind it at once. |
| **C. Single provider, manual recovery only** (status quo) | Keep one `rpc_url` per network; an operator manually re-runs `stellar tx status <hash>` after a failure. | Poor — any provider hiccup blocks all operations until a human intervenes. | N/A | None. | Lowest — nothing to build. | High — nothing stops an operator from re-running the original command out of habit after a timeout. |

Approach A is recommended for this repository's current scale (a handful of operators
running an admin CLI, plus scripts) because it removes the duplicate-submission risk without
adding a service to operate. Approach B is the right call if/when multiple independent
services (not just this repo's CLI) need shared failover — see Cross-repository impact.
Provider-specific SLA/latency/cost figures (e.g. comparing SDF's public RPC against specific
third-party hosted providers) are intentionally not asserted here as numbers: they drift
too fast for an ADR and belong in the operational runbook or a living doc that's actually
monitored, not committed as a one-time snapshot. The category-level comparison — SDF public
RPC (free, rate-limited, single operator's infra) vs. a self-hosted Soroban RPC node
(full control, requires running and syncing a node) vs. a third-party hosted provider
(no infra to run, but another party's uptime and rate limits apply) — is what Approach A's
failover list is designed around: it must tolerate any mix of these being configured.

## Alternatives considered

### Approach B: dedicated RPC gateway (see matrix)

Rejected for now, not permanently: it is strictly better once more than one Lafiya
component needs shared failover, but introduces an always-on service this repository has no
mechanism to deploy or monitor today. Revisit if `lafiya-web` or a verifier client needs the
same failover behavior — see Follow-up.

### Approach C: keep a single provider, formalize only the manual runbook

Rejected as insufficient on its own: it does not address "timeout and ambiguous-submit
behavior" from the acceptance criteria, and leaves the duplicate-submission risk exactly
where it is today. The runbook this ADR adds (`docs/runbooks/rpc-outage-recovery.md`) is
still valuable as the human-in-the-loop fallback for when the automated retry/poll budget in
Approach A is exhausted, so it is kept as a complement to Approach A, not a replacement.

### Treating every RPC failure as safe-to-retry

Rejected: this is the status quo bug. A generic `catch`/retry-on-any-error wrapper around
the current `stellar` CLI invocations would "fix" transient timeouts at the cost of turning
every ambiguous failure into a potential duplicate `attest()` call — worse than doing
nothing, since it actively creates duplicate on-chain records instead of just blocking.

## Consequences

### Positive

- Ambiguous RPC failures no longer default to either "silently drop the operation" or
  "blindly duplicate it" — both are replaced by poll-by-hash recovery with an explicit
  escalate-to-operator outcome when the budget is exhausted.
- The retry/unsafe-retry classification is enforced in code (`classify`, `FailoverClient`),
  not left as tribal knowledge a future contributor has to rediscover per call site.
- No new always-on infrastructure; `config/networks.toml` and the CLI/scripts remain the
  only moving parts, consistent with this repository's existing "one flag switches
  networks" design goal (`config/README.md`).

### Trade-offs and risks

- Multi-provider config means each network's operator now has to source and maintain more
  than one working RPC URL, instead of trusting the one SDF default.
- `backoff_schedule` has no jitter (documented in `src/lib.rs`); if the CLI is ever run by
  many concurrent automated callers against the same provider set, add jitter first to avoid
  a synchronized retry stampede after a shared provider's outage.
- The prototype's retry/poll budgets are static per-call constants; they are not informed by
  a provider's advertised transaction-retention window, so `max_poll_rounds` could in theory
  be exhausted before a slow-to-index provider would have answered. Follow-up work should
  tune the defaults against real provider behavior before this ships in the CLI.
- This ADR does not change `attest()`'s on-chain idempotency — it only prevents the *client*
  from creating a duplicate through ambiguous-failure resubmission. A caller that
  deliberately calls `attest()` twice for the same `record_hash` still gets two records; that
  is existing, intentional multi-attestation behavior, not a bug this ADR addresses.

## Follow-up

- Extend `config/networks.toml` and `crates/lafiya-config` to accept an ordered list of RPC
  endpoints per network (e.g. `rpc_urls = ["...", "..."]`), falling back to the existing
  single `rpc_url` key for backward compatibility.
- Wire `crates/lafiya-rpc-resilience`'s `FailoverClient` behind a real Soroban RPC
  `RpcProvider` implementation (HTTP + `getTransaction` polling) and use it from
  `crates/lafiya-cli` in place of the direct `std::process::Command::new("stellar")` calls in
  `crates/lafiya-cli/src/main.rs`.
- Add a `lafiya-cli tx status <hash>` subcommand so an operator (or the runbook) can poll a
  specific transaction hash directly, independent of a retry flow.
- Persist an in-flight transaction's hash and intent (which command, which network, which
  contract call) to local disk before submitting, so a crashed or interrupted CLI invocation
  can be resumed with `tx status` instead of leaving the operator to reconstruct what was in
  flight from memory.
- Tune `RetryPolicy` defaults, and add jitter to `backoff_schedule`, against observed
  behavior of the specific providers configured for `testnet`/`futurenet`/`mainnet`.
- Revisit Approach B (dedicated gateway) if `lafiya-web` or a verifier client needs the same
  failover behavior outside this repository's CLI/scripts.
- Operator alerting (e.g. paging when a recovery run reaches
  `RecoveryResult::ExhaustedNeedsOperator`) is out of scope for this ADR and depends on
  whatever alerting stack Lafiya's operations settle on; not designed here.

## References

- [Issue #132](https://github.com/Lafiya-xyz/Lafiya-contract/issues/132) — the spike this
  ADR resolves.
- [`crates/lafiya-rpc-resilience`](../../crates/lafiya-rpc-resilience) — failure-injection
  prototype, transaction-state model, and retry-classification implementation.
- [`docs/runbooks/rpc-outage-recovery.md`](../runbooks/rpc-outage-recovery.md) — the
  operator-facing recovery runbook this ADR's model is built to support.
- `config/networks.toml`, `config/README.md`, `scripts/lib/config.sh` — current
  single-provider network configuration this ADR proposes extending.
- `contracts/attester-registry/src/lib.rs:249,283,321` — `add_attester` /
  `add_attester_with_info` / `remove_attester` idempotency referenced in Context.
- `contracts/attestation-registry/src/lib.rs:294,310-323` — `attest`'s non-idempotent
  sequence-incrementing storage, referenced in Context.
