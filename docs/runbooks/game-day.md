# Runbook: Disaster-recovery game day

Rehearses the recovery runbooks under realistic pressure and measures time-to-recover. Run it on **testnet with the local sandbox tooling only**, never against mainnet or with production keys.

Related: [contract-upgrade.md](contract-upgrade.md), [rpc-outage-recovery.md](rpc-outage-recovery.md), [security-triage.md](security-triage.md), [watchdog.md](watchdog.md), [testnet-deployment.md](testnet-deployment.md), [chw-key-custody.md](../security/chw-key-custody.md). Report format: [game-day-report-template.md](game-day-report-template.md).

## Roles

| Role | Responsibility |
| --- | --- |
| Facilitator | Reads injects, controls the clock, answers "the world" questions; does not help solve |
| Operator(s) | Execute the runbooks with only the documented steps |
| Scribe | Timestamps every action in the report (UTC); records each place a runbook was wrong, missing, or unclear |

## Preparation (T-1 week)

1. `cargo xtask sandbox up --scenario pilot` for local dry runs; deploy a fresh testnet set with `scripts/deploy-testnet.sh` using throwaway keys funded from friendbot.
2. Confirm the facilitator has the inject commands below working, and operators have the repo at the current release tag and no extra notes.
3. Agree the **RTO target** per scenario (below) before the exercise, so results are judged against a pre-committed number.

## Scenarios

Clock starts at the inject; it stops at the **success criterion**. Record the start, detection, containment and recovery timestamps (T0–T3).

### S1: Key loss (admin / multisig signer)

| | |
| --- | --- |
| Inject | Facilitator deletes one signer key of the testnet admin multisig, then announces "the device holding signer 3 is gone and signer 2 is unreachable." Threshold is 2-of-3, so quorum is lost. |
| Expected runbook path | `chw-key-custody.md` (rotation, not seed restore); multisig recovery/rotation in `multisig-account`; two-step admin transfer in `contract-upgrade.md` |
| Success criteria | (a) A new admin/signer set controls the registries; (b) `get_admin` returns the new address on both registries; (c) a test attester add succeeds under the new admin; (d) the old key can no longer sign. |
| RTO target | 60 min |

### S2: RPC outage mid-operation

| | |
| --- | --- |
| Inject | Block the primary RPC host (hosts-file or firewall rule) while an operator is running `lafiya-cli attester add`, so the submission times out ambiguously. |
| Expected runbook path | `rpc-outage-recovery.md` (check the ledger before re-running; failover endpoint; poll `getTransaction`) |
| Success criteria | (a) The operator determines whether the transaction landed **without double-submitting**; (b) the attester state is correct exactly once; (c) the failover endpoint is used and documented. |
| RTO target | 30 min |

### S3: Contract bug requiring pause and fixed upgrade

| | |
| --- | --- |
| Inject | Facilitator supplies a "reported bug" (a reproducible wrong result in `attestation-registry`) and a fixed build. The watchdog alerts on the upgrade attempt. |
| Expected runbook path | `security-triage.md` → pause → `contract-upgrade.md` §2–5 (hash verification, `upgrade`, `migrate`, spot checks) → unpause; `watchdog.md` alert handling |
| Success criteria | (a) Contract paused before the fix is applied; (b) on-chain wasm hash equals the reviewed build; (c) post-upgrade spot checks pass; (d) contract unpaused; (e) watchdog `upgraded` alert acknowledged and matched to the announced upgrade. |
| RTO target | 90 min |

## Procedure

1. Facilitator briefs roles (10 min) and starts the shared clock.
2. Run S1, S2, S3 in order, resetting the testnet fixture between scenarios (`scripts/testnet_reset/seed-fixtures.sh` or a fresh deploy).
3. Hot wash immediately after each scenario (10 min): what blocked you, what was unclear.
4. Within 5 working days, publish the after-action report from the template under `docs/runbooks/game-day-reports/YYYY-MM-DD.md`.
5. Each failed or slow step gets a GitHub issue; link it in the report's "Failed steps" table. Runbook fixes go in a PR referencing that issue.

Use `scripts/game_day_timer.sh` to record timestamps (appends `UTC timestamp, scenario, phase` lines to a CSV the scribe pastes into the report).

## Recurring schedule and ownership

| Item | Value |
| --- | --- |
| Cadence | Twice a year (Q1 and Q3), plus after any major release that changes upgrade, key custody, or RPC handling |
| Owner | Security / release lead |
| Calendar | Owner books the exercise one quarter ahead and opens a tracking issue labelled `process` + `security` titled `Game day YYYY-Qn` |
| Rotation | Facilitator, operator, and scribe roles rotate each exercise; at least one operator who has not run the runbook before |
| Exit | Previous exercise's open runbook-fix issues must be closed or explicitly deferred before the next one |

## Runbook gaps identified while preparing (pre-exercise)

Found by reading the runbooks against the scenarios; confirm or refute during the exercise.

| # | Gap | Scenario |
| --- | --- | --- |
| G1 | No runbook for **admin/multisig quorum loss**. `chw-key-custody.md` covers CHW keys; `contract-upgrade.md` assumes a working admin. Recovery depends on a guardian-recovery path that `docs/` says is design-only (`verification/tla/MultisigRecovery.tla`). | S1 |
| G2 | No documented "emergency pause" step in the order of operations for a live bug; pause is described per contract but not sequenced before the fix. | S3 |
| G3 | No timing/measurement guidance, so RTOs are not comparable between exercises (addressed by this document and the timer script). | all |
