# Incident Management Framework

**Audience:** on-call operators, maintainers, the incident commander, and anyone
who has to tell the outside world that something is wrong.

**Scope:** how we declare, run, communicate, and learn from an incident on the
Lafiya contracts, indexer, relayer, and the tooling that operates them. The
technical procedures live in the [runbooks](../runbooks/); this document is the
layer above them: who is in charge, how bad it is, who gets told, and what we
write afterwards.

> ⚠️ Patient safety comes before uptime, and uptime comes before tidiness. If a
> step in a runbook conflicts with the severity matrix below, the matrix wins and
> the runbook gets fixed afterwards.

---

## 1. Severity matrix

Severity is set by **patient-safety impact first**, then by data integrity, then
by availability. When two levels both seem to fit, take the higher one; downgrade
only with the incident commander's explicit call, recorded in the incident log.

| Level | Patient-safety impact | Data / integrity impact | Availability impact | Declare within | Update cadence | Postmortem |
| --- | --- | --- | --- | --- | --- | --- |
| **SEV1** | A verified or suspected **wrong verification verdict** reaches a patient or CHW (a revoked or fraudulent attester's attestation verifies, or a valid one fails), or health data is exposed. | Attestation or attester state is corrupted, or an unauthorized party can write it. | Registries unusable on mainnet; no workaround. | 15 min | every 30 min | required, within 5 business days |
| **SEV2** | Verdicts are still correct but a whole class of patients/CHWs cannot be verified or paid (e.g. one registry paused, claims failing). | Drift or a reconciliation gap with no known corruption; a single contract's state suspect. | Major function degraded on mainnet; workaround exists but is manual. | 30 min | every 1 h | required, within 10 business days |
| **SEV3** | No patient-facing impact yet; a control that protects patients is degraded (watchdog blind, TTL keeper down, canary failing). | Testnet/staging state wrong; indexer lag or gaps. | Degraded non-critical path, or mainnet unaffected. | 4 h (business hours) | daily | required for repeat/recurring, otherwise optional |
| **SEV4** | None. | None. | Cosmetic, docs, or a single failed CI job. | next business day | as needed | not required |

**Escalation triggers** — raise the severity immediately when any of these is
true:

- A patient or CHW reports a wrong verdict in the field.
- The watchdog pages on a privileged event nobody can explain.
- `reconcile()` reports negative drift on the incentive pool.
- A key, sponsor account, or webhook secret is suspected compromised.
- Two or more SEV3s share a root cause.

**De-escalation** requires the incident commander to state, in the incident log,
which patient-safety condition is now false.

---

## 2. Roles

Every incident has exactly one person per role. One person may hold several
roles on a SEV3/SEV4; on a SEV1 the first three must be different people.

| Role | Owns | Does not do |
| --- | --- | --- |
| **Incident commander (IC)** | Declares severity, assigns roles, decides pause/upgrade/rollback, runs the timeline, calls de-escalation and closure. | Debug or type commands. The IC keeps their hands off the keyboard. |
| **Operations lead** | Executes the runbook steps (pause, upgrade, reconcile, rotate, failover) and reports each result back to the IC. | Talk to anyone outside the incident channel. |
| **Communications lead** | Owns the status page, CHW/partner messages, and donor notices; keeps the approved-message log. | Speculate on cause or give ETAs the IC has not approved. |
| **Scribe** | Keeps the incident log: timestamps, decisions, who decided, links to evidence. | Investigate. |
| **Subject-matter expert(s)** | Contract, indexer, relayer, or key-ceremony expertise as needed. | Own the incident. |
| **Executive sponsor** | Informed on SEV1/SEV2; approves donor and regulator-facing statements and any spend outside the normal cap. | Direct the technical response. |

The IC is whoever is on call when the page lands, until they hand over
explicitly ("IC handover to <name> at <time>"). Handover is written in the log.

---

## 3. Lifecycle

1. **Detect.** A page (watchdog, canary, TTL keeper, indexer reconcile), a
   failed workflow, or a human report. The person who notices opens the incident
   channel and, if unsure, declares at the level the matrix suggests.
2. **Declare.** IC states severity, roles, and a one-line impact statement in
   the channel. Start the log.
3. **Stabilise.** Prefer the reversible action: `pause()` the affected registry,
   stop the relayer, freeze the sponsor account. Correctness before recovery.
4. **Communicate.** Communications lead publishes the first status update within
   the cadence for the severity, using the templates in §4.
5. **Resolve.** The condition that caused the declaration is gone and verified
   (canary green, `reconcile()` clean, verdicts spot-checked). IC declares
   resolved and states the residual risk.
6. **Review.** Postmortem per §5, action items tracked to closure.

---

## 4. Communications templates

Rules that apply to every template:

- Never include patient identifiers, wallet addresses tied to a person, or
  exploit details. Say what is affected, what we did, and what the reader should
  do.
- Every message is approved by the IC before it goes out; the communications
  lead records the approved text and time in the incident log.
- Localise CHW-facing messages into the languages the CHW network uses; keep
  sentences short enough to survive SMS.

### 4.1 Status page

```
[SEV<n>] <short title> — <investigating | identified | monitoring | resolved>

Impact: <who is affected and how, in one sentence>
Started: <UTC timestamp>
Current status: <what we know now>
What we are doing: <current action>
What you should do: <action or "no action required">
Next update: <UTC timestamp>
```

### 4.2 SMS / WhatsApp to CHWs

```
LAFIYA SERVICE NOTICE (<date>)
<What is not working, in one sentence.>
<What to do instead, in one sentence.>
Do not retry <action> until we confirm. We will message you again by <time>.
```

### 4.3 Donor notice

```
Subject: Lafiya incident <id> — <SEV<n>> — <status>

Summary: <two sentences: what happened, what is affected>
Patient impact: <none | describe; state whether any verdict was wrong>
Funds: <whether pool custody, sponsor balance, or payouts are affected>
Actions taken: <pause/upgrade/rotation, with timestamps>
Next update: <date and channel>
Contact: <named communications lead>
```

### 4.4 Internal page (watchdog / on-call)

```
SEV<n> declared by <name> at <UTC>.
Impact: <one line>
Runbook: <link>
Incident channel: <link>
Roles: IC <name>, Ops <name>, Comms <name>, Scribe <name>
```

---

## 5. Blameless postmortem template

Copy this into `docs/process/postmortems/<YYYY-MM-DD>-<short-name>.md`.

```markdown
# Postmortem: <title>

- Incident id / channel:
- Severity at declaration / at closure:
- Date and duration (UTC):
- Roles: IC, Ops, Comms, Scribe, SMEs
- Runbooks used:

## Summary
Two or three sentences a non-engineer can follow.

## Patient-safety impact
Was any verdict wrong, delayed, or unavailable? For how long, and how do we know?

## Timeline
| UTC | Event | Who |
| --- | --- | --- |

## What went well

## What went badly

## Root cause(s)
Technical and organisational. "Human error" is not a root cause.

## Detection
How did we find out? Would the watchdog/canary have caught it alone?

## Action items
| # | Action | Type (prevent/detect/mitigate) | Owner | Due | Issue | Status |
| --- | --- | --- | --- | --- | --- | --- |

## Lessons for the framework
Changes needed to the severity matrix, roles, templates, or runbooks.
```

**Action-item tracking.** Every action item gets an owner, a due date, and a
tracking issue before the postmortem is merged; an item without all three is not
an action item. Items are reviewed at the next ops sync until closed, and a
SEV1/SEV2 postmortem is not closed while any *prevent* or *detect* item is open
without a written, IC-approved deferral.

---

## 6. Runbook severity defaults

Each runbook declares its default severity and roles in its header. The table
below is the index; the runbook header is authoritative.

| Runbook | Default severity | IC | Operations lead | Comms lead |
| --- | --- | --- | --- | --- |
| [contract-upgrade](../runbooks/contract-upgrade.md) | SEV2 (SEV1 if a wrong verdict is observed) | on-call maintainer | release manager | maintainer on rotation |
| [incentive-pool-drift](../runbooks/incentive-pool-drift.md) | SEV2 (SEV1 if negative drift persists after pause) | on-call maintainer | treasury operator | communications lead |
| [rpc-outage-recovery](../runbooks/rpc-outage-recovery.md) | SEV2 (SEV3 if a failover provider absorbs it) | on-call maintainer | on-call operator | communications lead |
| [key-ceremony](../runbooks/key-ceremony.md) | SEV1 for a suspected compromise; SEV3 for a planned ceremony | ceremony lead | key holders | communications lead |
| [admin-key-rotation](../runbooks/admin-key-rotation.md) | SEV1 for compromise-driven rotation; SEV3 for scheduled | on-call maintainer | ceremony lead | communications lead |
| [attester-fraud-response](../runbooks/attester-fraud-response.md) | SEV1 if a flagged attester's attestations verify; otherwise SEV2 | on-call maintainer | fraud reviewer | communications lead |
| [security-triage](../runbooks/security-triage.md) | SEV1 for Critical/High with patient impact; SEV2 for Medium; SEV3 for Low | security rotation maintainer | report owner | communications lead |
| [watchdog](../runbooks/watchdog.md) | SEV1 for an unexplained privileged event; SEV3 for watchdog blindness | on-call maintainer | on-call operator | communications lead |
| [canary-attestations](../runbooks/canary-attestations.md) | SEV2 when the canary fails or exceeds latency; SEV3 for a single flake | on-call maintainer | on-call operator | communications lead |
| [ttl-keeper](../runbooks/ttl-keeper.md) | SEV3 (SEV2 if an entry has already archived) | on-call maintainer | on-call operator | communications lead |
| [relayer-operations](../runbooks/relayer-operations.md) | SEV2 for sponsor exhaustion or relayer outage; SEV1 for sponsor key compromise | on-call maintainer | relayer operator | communications lead |
| [chw-trustline-onboarding](../runbooks/chw-trustline-onboarding.md) | SEV3 (SEV2 if a cohort cannot be paid) | on-call maintainer | support operator | communications lead |
| [sdk-protocol-upgrade](../runbooks/sdk-protocol-upgrade.md) | SEV3 | on-call maintainer | release manager | communications lead |
| [testnet-deployment](../runbooks/testnet-deployment.md) | SEV4 (SEV3 if staging is blocked) | release manager | release manager | none |
| [testnet-reset](../runbooks/testnet-reset.md) | SEV4 (SEV3 if the automation fails to recover) | release manager | on-call operator | none |
| [mainnet-readiness](../runbooks/mainnet-readiness.md) | SEV3 | release manager | release manager | communications lead |

---

## 7. Tabletop exercises

The framework is exercised at least twice a year, and after any SEV1. A tabletop
is a discussion, not a drill: no production access, no real paging.

1. Pick a scenario from the runbook table above (rotate the choice).
2. Assign roles to people who do not normally hold them.
3. Walk the lifecycle in §3 against the real runbook, using the real templates.
4. Record notes in `docs/process/tabletops/<YYYY-MM-DD>-<scenario>.md`: what
   worked, what was missing, and action items in the §5 format.
5. Feed the action items into the same tracker as postmortem items.

See [the first tabletop notes](tabletops/2026-11-20-sev1-wrong-verdict.md) for
the format.
