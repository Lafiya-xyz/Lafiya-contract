# Game day after-action report: YYYY-MM-DD

Procedure: [game-day.md](game-day.md). Copy to `docs/runbooks/game-day-reports/YYYY-MM-DD.md`. Times are UTC; RTO = T3 − T0.

| Field | Value |
| --- | --- |
| Environment | testnet (contract IDs, release tag) |
| Facilitator / operators / scribe | |
| Tooling versions | `stellar` CLI, `lafiya-cli`, sandbox scenario |

## Results

| Scenario | T0 inject | T1 detected | T2 contained | T3 recovered | **Measured RTO** | Target RTO | Met? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| S1 Key loss | | | | | | 60 min | |
| S2 RPC outage | | | | | | 30 min | |
| S3 Contract bug | | | | | | 90 min | |

## Timeline

Paste the timer CSV (`scripts/game_day_timer.sh` output) per scenario.

## Failed or slow steps

Every row links a GitHub issue (and the fixing PR once it exists).

| Scenario | Runbook / step | What went wrong | Time lost | Issue |
| --- | --- | --- | --- | --- |

## What worked

## Runbook changes made

## Follow-ups for next exercise

Next exercise date, owner, and open items carried over:
