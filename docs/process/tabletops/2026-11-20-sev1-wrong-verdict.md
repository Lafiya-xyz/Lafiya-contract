# Tabletop: SEV1 — wrong verification verdict in the field

- Date: 2026-11-20 (UTC), 90 minutes
- Framework: [incident management](../incident-management.md)
- Scenario: a CHW reports that an attestation from a suspended attester still
  verifies in `lafiya-web`; a second report follows from another district.
- Runbooks exercised: [attester-fraud-response](../../runbooks/attester-fraud-response.md),
  [watchdog](../../runbooks/watchdog.md), [contract-upgrade](../../runbooks/contract-upgrade.md)

## Roles

| Role | Assigned to |
| --- | --- |
| Incident commander | maintainer A (does not normally hold IC) |
| Operations lead | maintainer B |
| Communications lead | maintainer C |
| Scribe | contributor D |
| SME (attester-registry) | maintainer E |

## Walkthrough notes

1. **Declare.** The matrix put this at SEV1 on the first report (wrong verdict
   reaching a patient). The 15-minute declare target was met in the exercise;
   the IC had to look up the escalation triggers, which cost about two minutes.
2. **Stabilise.** Ops paused `attestation-registry` first, then
   `attester-registry`. The pause policy allows the guardian to pause alone, so
   no multisig round-trip was needed. Good.
3. **Communicate.** The status-page template was usable as written. The CHW SMS
   template needed a field for "which app screen to use instead" — added to the
   template after the exercise.
4. **Diagnose.** The watchdog had paged on the suspension event but nobody had
   linked it to the verification path. The runbook table's default severity for
   `watchdog` (SEV1 for an unexplained privileged event) would have escalated
   earlier had it been followed.
5. **Resolve.** Canary green and a spot-check of ten verdicts confirmed the fix.
   De-escalation required the IC to state which patient-safety condition was
   false; that worked as intended.

## What worked

- Severity matrix gave an unambiguous SEV1 call without debate.
- Pause-before-diagnose ordering was followed naturally.
- Templates were short enough to fill in live.

## What was missing

- No named backup IC; the exercise stalled when the IC's laptop dropped.
- The donor-notice template had no field for "funds unaffected", which is the
  most common answer and the one donors ask for first.
- No agreed place to record the approved-message log other than the incident
  channel.

## Action items

| # | Action | Type | Owner | Due | Issue | Status |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Add a named backup IC to the on-call rotation and to §2 | prevent | maintainer A | 2026-12-05 | #772 follow-up | open |
| 2 | Add "which screen to use instead" to the CHW SMS template | mitigate | maintainer C | 2026-12-05 | #772 follow-up | done |
| 3 | Add an explicit "funds unaffected" line to the donor notice | mitigate | maintainer C | 2026-12-05 | #772 follow-up | done |
| 4 | Define the approved-message log location in §4 | detect | maintainer B | 2026-12-12 | #772 follow-up | open |
| 5 | Schedule the next tabletop (SEV2 incentive-pool drift) | prevent | maintainer A | 2027-05-20 | #772 follow-up | open |
