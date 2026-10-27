# Runbook: Security Watchdog (`lafiya-watchdog`)

The watchdog follows privileged contract events in real time and pages on-call.
Several controls (two-step admin transfer, upgrade review, recovery delays)
only work if someone notices the first step; this is that someone.

## Deployment

1. Build: `cargo build --release -p lafiya-watchdog`.
2. Copy [`crates/lafiya-watchdog/watchdog.example.toml`](../../crates/lafiya-watchdog/watchdog.example.toml)
   to `watchdog.toml` and set `rpc_url`, `contracts`, `release_manifests`, and
   `sinks`. Keep webhook URLs and API keys out of git (template the file from
   your secret store).
3. Run it as a supervised service (systemd `Restart=always`, a Kubernetes
   Deployment with one replica, ...):

   ```bash
   lafiya-watchdog --config /etc/lafiya/watchdog.toml run
   ```

4. Configure the **dead-man's switch**: set `heartbeat.ping_url` to a
   healthchecks.io / Cronitor check with a grace period of about 5 minutes and
   route that check to the same on-call rotation. The watchdog pings only after
   healthy polls, so a crashed host, a stuck process, or a lost network all page.
   See [Heartbeat / dead-man's switch](#heartbeat--dead-mans-switch) for a
   step-by-step Healthchecks.io setup.
5. Use a different RPC provider from the one the admin tooling uses, so one
   provider outage cannot blind both.
6. Point `state.path` at a **persistent volume** (not `tmpfs`, not the container
   layer). The watchdog stores rule window state and last-alert fingerprints
   there; losing it re-opens the duplicate/lost-alert window described below.

## Persistent state and restart semantics

The watchdog keeps two things on disk under `state.path`:

- **Rule window state** — the last processed ledger, per-rule sliding windows
  (e.g. the `mass_enrollment` / `bulk_revocation` counters), and in-flight
  windows that have not yet been flushed to a sink.
- **Last-alert fingerprints** — a hash of each alert (rule, contract, ledger,
  payload) that was already delivered, so a restart does not re-page for events
  that were already sent.

On startup the watchdog loads this state and resumes from the last processed
ledger. Writes are flushed before a sink is acknowledged, so a `kill -9` at any
point leaves either "alert not yet sent" (it will be sent after restart) or
"alert sent and fingerprinted" (it will not be re-sent) — never both.

Operational rules:

- **One replica only.** Two watchdogs sharing a state directory will fight over
  the state file. Scale by adding rules, not replicas.
- **Do not delete `state.path`** to "reset" the watchdog. If you must re-alert
  on a range, use `replay --send` explicitly; that bypasses dedup by design.
- **Back up the state directory** before upgrades so you can roll back without
  replaying from an old ledger.
- If the state file is corrupt, the watchdog refuses to start rather than
  silently re-alerting. Restore from backup, or move the file aside and accept
  that recent alerts may repeat.

### Kill -9 test

The acceptance test for restart safety is a hard kill, not a graceful stop:

```bash
# start the watchdog, let it process a few ledgers, then:
kill -9 "$(pgrep -f 'lafiya-watchdog.*run')"
# restart with the same state.path and confirm:
#   - no alert already delivered before the kill is delivered again
#   - no event between the last processed ledger and the kill is lost
lafiya-watchdog --config /etc/lafiya/watchdog.toml run
```

Run this against a local quickstart with a scripted event sequence before
trusting a new release in production.

## Deduplication, grouping, and escalation

- **Deduplication** — identical alerts (same rule, contract, ledger, payload)
  are collapsed using the persisted fingerprints. A flapping condition pages
  once, not once per poll.
- **Grouping** — alerts that arrive within the grouping window are batched into
  a single notification per sink, so a burst of `bulk_revocation` events is one
  page with a count, not hundreds.
- **Escalation** — alerts are routed by severity and escalate if not
  acknowledged within the configured ack timeout:

  | Severity | Initial route | Ack timeout | Escalation |
  | --- | --- | --- | --- |
  | critical | P1 page + security channel | 5 min | Re-page primary, then secondary on-call, then incident commander |
  | high | Security channel + email | 30 min | Re-notify security channel, then on-call |
  | info | Ops channel | none | No escalation |

  Acknowledging an alert (via the sink's ack mechanism or the `command` sink)
  stops escalation. An unacknowledged critical alert keeps escalating until
  someone owns it.

## Heartbeat / dead-man's switch

The heartbeat is the only signal that distinguishes "quiet chain" from "dead
watchdog". The watchdog sends a periodic ping to an external monitor **only
after healthy polls**; if the pings stop, the monitor pages.

### Healthchecks.io setup

1. Create a check in Healthchecks.io:
   - **Period**: 1 minute (match `heartbeat.interval`).
   - **Grace time**: 5 minutes.
   - **Integrations**: the same PagerDuty / Opsgenie service and security
     channel webhook used for `critical` alerts.
2. Copy the check's ping URL into `heartbeat.ping_url` in `watchdog.toml`
   (template it from your secret store; the URL is a bearer secret).
3. Optionally set `heartbeat.fail_url` (Healthchecks.io `/fail`) so the
   watchdog can report a failing poll immediately instead of waiting for the
   grace period to expire.
4. Verify: stop the watchdog, wait past the grace period, and confirm the
   monitor pages the on-call rotation. Then restart and confirm the check goes
   green again.

Expected behavior on absence: if the watchdog crashes, hangs, loses network, or
its RPC provider goes down, pings stop and Healthchecks.io pages after the grace
period. This is the backstop for `watchdog_lag` and `watchdog_failing` — those
alerts only fire if the watchdog is alive enough to send them.

## Guarded auto-response (pause proposals)

On `critical` rules the watchdog **proposes** a pause; it never executes one.
Fully automatic pausing would be an attack surface (spoofed alerts causing a
DoS), so containment stays behind guardian signatures.

Flow:

1. A critical alert fires and passes the **signed-alert authenticity check**
   (the alert must carry a valid signature from the event source / configured
   signer; unsigned or invalid alerts never produce a proposal).
2. The watchdog checks the **pause-policy matrix** in
   [`docs/PAUSE-POLICY.md`](../PAUSE-POLICY.md): paths excluded by policy are
   never proposed for pause, regardless of severity.
3. The watchdog applies its **rate limit** on proposals (per contract and
   globally) so a flood of alerts cannot induce a flood of pause proposals.
4. The watchdog builds the **unsigned** pause transaction and auth entries and
   attaches them to the alert. It never holds guardian keys.
5. A guardian reviews the proposal and signs it through the multisig tooling
   (see [`docs/adr/0012-coordinated-emergency-stop.md`](../adr/0012-coordinated-emergency-stop.md)).
   The proposal is only submitted once the required guardian signatures are
   collected.

Threat notes (spoofing and replay):

- **Spoofing** — proposals are only built from alerts that pass the signed-alert
  authenticity check; an attacker who can post to a webhook cannot induce a
  pause proposal.
- **Replay** — proposals are bound to the alert fingerprint and ledger range;
  replaying an old alert does not produce a fresh, submittable proposal.
- **DoS via auto-pause** — the rate limit and the guardian-signature requirement
  mean a spoofed or replayed alert cannot pause the chain on its own.

The sandbox end-to-end demo (malicious event → ready proposal) must complete in
under 60 seconds; see the watchdog crate tests for the scripted scenario.

## Recommended on-call routing

| Severity | Sink | Response |
| --- | --- | --- |
| critical | PagerDuty / Opsgenie (P1) **and** the security channel webhook | Page immediately, 24/7 |
| high | Security channel webhook, email via `command` sink | Acknowledge within business hours |
| info | Webhook to an ops channel | No action; audit trail |

Every privileged action should be announced in the security channel before it
is submitted. An alert with no matching announcement is an incident.

## Alert types

| Kind | Severity | Meaning | First response |
| --- | --- | --- | --- |
| `admin_transferred` | critical | Admin changed on a registry | Confirm with the signer set that this was planned; if not, treat the admin as compromised, pause via the remaining authority and start incident response |
| `attester_registry_repointed` | critical | Attestation registry now trusts a different attester registry | Verify the new address against the deployment record; an unknown target means attestations can be forged |
| `upgraded` | critical | Contract wasm upgraded | Match against the announced upgrade ([contract-upgrade.md](contract-upgrade.md)) |
| `unknown_wasm_upgrade` | critical | Upgrade target hash is in no release manifest | Assume compromise until the hash is traced to a reviewed build |
| `wasm_changed_without_event` | critical | On-chain wasm hash changed with no `upgraded` event | Treat as compromise or a watchdog blind spot; investigate immediately |
| `multisig_reconfigured` | critical | Multisig signer set, threshold, or recovery changed | Confirm with every signer out of band |
| `pause_toggled` | high | Registry paused or unpaused | Confirm with the admin; unplanned unpause during an incident is critical |
| `probation_breach` | high | A probationary attester breached a limit (`ProbationBreach`) | Review its attestations since `ProbationStarted`; promote only after review ([spec](../specs/attester-probation.md)) |
| `attester_slashed` | high | A bond slash executed (`SlashExecuted`) | Confirm the case evidence and consider suspending or removing the attester |
| `mass_enrollment` | high | More than N attesters added within M ledgers | Check the batch against the approved attester list |
| `bulk_revocation` | info | Attestation revoked | Watch for bursts |
CI runs `every_critical_rule_reaches_the_webhook`, which feeds the same event
sequence through the rules and a real HTTP webhook sink into a mock server.

## Incentive-pool rules

The pool rules read the same event stream as the registry rules; the pool
contract address is configured under `contracts` alongside the registries.
Thresholds live in the `[pool]` section of `watchdog.toml`:

| Key | Default | Meaning |
| --- | --- | --- |
| `large_withdrawal_threshold` | `10000000000` (stroops) | Withdrawals at or above this amount raise `large_pool_withdrawal` |
| `approval_burst_count` | `10` | Number of approvals that constitutes a burst |
| `approval_burst_ledgers` | `100` | Window (in ledgers) over which approvals are counted |
| `readd_window_ledgers` | `1000` | How far back to look for a remove/re-add of the claiming attester |

Tune the thresholds to the pool's normal payout size before enabling the
rules; a threshold far below typical payouts will page on every cycle. The
`claim_by_readded_attester` rule keeps a bounded in-memory history of attester
removals, so after a watchdog restart it can only flag re-adds that happened
since startup; replay over an exported event range covers the gap.
| `attestation_expiring` | high | An attestation's live-until ledger is within the configured horizon (default 30 days) | Notify the attester and the patient; anyone may extend it (see below) |
| `attestation_archived` | high | An attestation entry has archived (rent lapsed) | Restore it with the CLI helper (see below) and extend its TTL |
| `attestation_disputed` | high | A patient flagged an attestation as disputed | Surface the dispute to verifiers; only the attester or admin can clear it (see below) |
| `dispute_spam` | high | More than N disputes from the same signer within M ledgers | Treat as abuse; review the signer and consider admin clearing |
| `watchdog_lag` | critical | Watchdog is behind the network by more than `max_lag_ledgers` | Check RPC health ([rpc-outage-recovery.md](rpc-outage-recovery.md)); nothing is being watched |
| `watchdog_failing` | critical | Polls failing repeatedly | Same as above |
| `heartbeat_missed` | critical | External monitor stopped receiving pings | The watchdog itself is down or stuck; check the host and process, then the RPC provider |
| `pause_proposal` | critical | An unsigned pause proposal is attached to a critical alert | Review against [`PAUSE-POLICY.md`](../PAUSE-POLICY.md), then collect guardian signatures via the multisig tooling |

## Attestation expiry and TTL extension

Soroban entries archive when their rent lapses. Attestations backing a patient's
card must not depend on the original attester remembering to extend them, so
`extend_attestation_ttl(record_hash, extend_to)` is **permissionless**: any
caller may extend an entry up to the contract's maximum horizon. It can never
shorten a TTL or alter attestation data.

Monitoring guidance:

- The watchdog emits `attestation_expiring` for every live attestation whose
  `live_until_ledger` falls inside `attestation_expiry_horizon_ledgers`
  (default: 30 days of ledgers). Route it to the attester's contact and the
  patient's channel so either can act.
- Indexers should expose `live_until_ledger` per record hash and alert on the
  same horizon; the watchdog is a backstop, not the only signal.
- Track the rent cost per attestation at scale (see
  [storage-cost.md](../storage-cost.md)) and budget the extension spend; a
  watchdog-driven extension job should be funded and rate-limited.

### Restoring archived entries

If an entry has already archived, restore it before extending:

```bash
# Inspect the record and its current TTL state
lafiya-cli attestation show --record-hash <hash>

# Restore the archived entry (pays rent) and extend it to the horizon
lafiya-cli attestation restore --record-hash <hash> --extend-to <ledger>
```

`restore` is idempotent: it is a no-op for entries that are still live. After a
restore, re-run `show` and confirm `live_until_ledger` is past the horizon, then
record the action in the security channel.

## Disputes

A patient can flag an attestation as disputed by signing the commitment with
the patient signature scheme from the QR spec. The dispute is an overlay keyed
by `(record_hash, signer)`; it never deletes or mutates the attestation.

- Verification returns a dedicated `Disputed` state. Verifiers **must** surface
  it and must not treat a disputed attestation as valid until it is cleared.
- Clearing requires the attester or an admin; the patient cannot clear it.
- Disputes are replay-safe via domain separation in the signed payload; see
  [verification-model.md](../specs/verification-model.md).
- Disputes add a linkability signal (a patient key tied to a record hash).
  Note this in the privacy review and avoid logging dispute payloads beyond
  what the alert needs.

First response to `attestation_disputed`: confirm the signer is the patient on
the record, notify the attester, and follow the responder guidance in the
verification model. For `dispute_spam`, escalate to the admin for review.

## Replay (forensics)

Evaluate the same rules over a historical range; alerts print as JSON lines and
are only sent with `--send`:

```bash
lafiya-watchdog --config watchdog.toml replay --from 1200000 --to 1210000
lafiya-watchdog --config watchdog.toml replay --events exported-events.jsonl
```

RPC retains events for a limited window (about 7 days on public providers);
older ranges need an archive RPC or an indexer export.

## Testing against a local quickstart

```bash
stellar container start local
# deploy the registries, then run admin transfer, upgrade, and repoint once each
LAFIYA_WATCHDOG_RPC=http://localhost:8000/rpc \
LAFIYA_WATCHDOG_CONTRACTS=C...,C... \
cargo test -p lafiya-watchdog -- --ignored live_quickstart_scenario
```

CI runs `every_critical_rule_reaches_the_webhook`, which feeds the same event
sequence through the rules and a real HTTP webhook sink into a mock server.

Restart safety is covered by the kill -9 test above; run it locally against the
quickstart before promoting a release. The guarded auto-response demo
(malicious event → ready pause proposal in under 60s) is exercised by the
watchdog crate's sandbox scenario tests.
