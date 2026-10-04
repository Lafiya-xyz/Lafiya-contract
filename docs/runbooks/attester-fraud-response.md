# Runbook: Attester Fraud/Anomaly Flag Response

**Audience:** whoever operates the fraud-scoring job from
[`crates/lafiya-fraud-scoring`](../../crates/lafiya-fraud-scoring) (issue #415) and whoever
reviews its output.

**Scope:** what to do with a `RiskReport` once the scoring job produces one. Not a description
of the scoring itself (see
[`docs/spikes/0415-attester-fraud-scoring.md`](../spikes/0415-attester-fraud-scoring.md)) or of
suspension mechanics generally (see `contracts/attester-registry`'s `suspend_attester`/
`reinstate_attester`, used in step 3 below).

## The pipeline: flag → review → suspend → (optional) compromise marking

### 1. Flag

The scoring job (run on a schedule once it is wired into an indexer -- see the design doc's
"Serving and alerting") produces a `RiskReport` per attester. Only act on `Severity::High`
through this runbook; `Severity::Medium` is a single independent signal and is logged for
awareness, not escalated, unless a reviewer notices a pattern across several `Medium` reports
for the same attester over time (see step 2).

### 2. Review

A human reviewer reads the `High` report's `reasons` (every point in `score` has one) and:

1. Confirms the flag isn't an artifact of a known, benign situation -- e.g. a documented
   staffing reassignment covering two regions temporarily (region mismatch), or a legitimate
   bulk backfill after an outage (velocity/burst). If so, record why the flag was overridden
   (attester, ledger range, reason) and stop here -- do not suspend.
2. Otherwise, spot-checks a sample of the flagged attester's `record_hash`es against
   `lafiya-web`'s off-chain record for that region, looking for the concrete failure mode the
   reasons named (e.g. for a churn flag, whether the repeated hash corresponds to a real
   re-verification workflow or an obvious scripted resubmission).
3. Decides: suspend now, or escalate for further investigation before suspending (e.g. if the
   evidence is suggestive but not conclusive, and the attester's continued activity in the
   meantime is an acceptable risk).

### 3. Suspend, with a reason code

`attester-registry`'s `suspend_attester(attester)` takes only the address -- there is no
on-chain reason field (see `contracts/attester-registry/src/lib.rs`). Record the reason
alongside the suspension in the ledger this repo already uses for durable operational history:

```sh
cargo run -p lafiya-cli -- --network testnet deployments record \
  --event admin_transfer --contract-kind attester-registry --contract-id <id> \
  --tx-hash <suspend tx hash> --operator "<reviewer identity>"
```

(`admin_transfer` is the closest existing event kind for an out-of-band admin action recorded
by hand -- see `deployments/README.md`. A dedicated `suspend` event kind, with a structured
reason-code field, is a natural follow-up to `deployments/schema.json` once this playbook is
exercised for real and a fixed reason-code vocabulary is worth codifying.) Suggested reason
codes to record in the operator/commit-message text until then: `VELOCITY`, `BURST`, `CHURN`,
`REGION_MISMATCH`, `REVOCATION_RATIO`, `COLLUSION`, or `MULTIPLE` (matching the `RiskReport`
reasons that drove the decision), plus `MANUAL` for a suspension not driven by this scoring job
at all.

Then submit the suspension itself, same as any other admin call:

```sh
cargo run -p lafiya-cli -- --network testnet attester ... # see lafiya-cli --help;
# suspend_attester is invoked the same way add_attester/remove_attester are, via the
# stellar CLI wrapper -- see crates/lafiya-cli/src/main.rs.
```

### 4. Optional: compromise marking

If review concludes the attester's *credentials* were compromised (rather than the attester
themself acting fraudulently) -- e.g. a leaked signing key used to submit scripted
attestations -- suspension still applies (their key can no longer add attestations), but two
follow-ups don't have on-chain support today and should happen off-chain:

1. Flag every attestation this attester made in the burst/anomaly window for **manual patient
   re-verification** -- a compromised key's attestations are not necessarily fraudulent
   individually, but their provenance is no longer trustworthy.
2. If the attester is later re-licensed under a new key, do **not** reinstate the old address
   via `reinstate_attester` -- allowlist the new address separately via `add_attester`, so the
   compromised key's suspension is permanent and auditable.

## Escalation

If a `High` flag's evidence is ambiguous even after step 2's spot-check, or the volume of
attestations at risk is large enough that suspension itself has a significant operational
impact (e.g. the attester covers a region with no other active attester), escalate to the
repo's maintainers before suspending rather than deciding unilaterally -- the same
"don't guess, hand it to a human with full context" principle
[`docs/runbooks/rpc-outage-recovery.md`](rpc-outage-recovery.md) uses for ambiguous transaction
outcomes.
