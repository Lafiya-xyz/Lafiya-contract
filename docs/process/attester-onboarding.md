# Attester Onboarding and Identity-Proofing Workflow

Owner: governance/legal owner (sign-off required; see
[Review status](#review-status)). Companion documents:
[CHW key custody](../security/chw-key-custody.md),
[attester fraud response](../runbooks/attester-fraud-response.md).

On-chain enrollment is only as trustworthy as the process behind it. Personal
data stays off chain; the chain carries only a hash of the evidence bundle and
a reason code.

## 1. Workflow

| Step | Actor | Output | Contract operation |
| --- | --- | --- | --- |
| 1. Application | Applicant | [Application form](templates/application-form.md) | none |
| 2. Document check | Reviewer | [Evidence checklist](templates/document-check.md); two-person check of licence and identity | none |
| 3. Interview | Reviewer + second reviewer | [Interview record](templates/interview-record.md) | none |
| 4. Decision | Governance panel | [Decision record](templates/decision-record.md) with reason code | none |
| 5. Enrollment | Regional registrar | Evidence bundle hash recorded | `add_attester_with_info(registrar, attester, license_hash, region, valid_from, valid_until)` |
| 6. Periodic review | Reviewer | Review record every 12 months, and on any fraud flag | `update_attester_info`, `suspend_attester_with_reason`, `remove_attester`, `rotate_attester` |

The attester generates its key on device per
[CHW key custody](../security/chw-key-custody.md) before step 5; the key
attestation is part of the evidence bundle.

## 2. Evidence bundle and on-chain reference

1. Collect the completed templates and supporting documents into one archive
   held in access-controlled storage with a retention period set by the legal
   owner.
2. Compute `license_hash = SHA-256(archive)`. Record the hash, the archive
   location, and the decision in the decision record.
3. Pass `license_hash` to `add_attester_with_info`. The hash proves the bundle
   was not altered after the decision and lets an auditor match chain state to
   evidence. It reveals no personal data.
4. Suspensions use `suspend_attester_with_reason` with a Symbol reason code
   from the list below. Never put names, identifiers, or free text on chain.

| Reason code | Meaning |
| --- | --- |
| `onboarded` | Approved after full process (decision record only) |
| `fraud_flag` | Suspended pending review after a fraud/anomaly flag |
| `key_compromise` | Key lost or compromised; pair with `rotate_attester` |
| `lapsed_review` | Periodic review overdue or failed |
| `licence_expired` | Underlying licence expired or revoked |
| `administrative` | Default used by `suspend_attester` |

## 3. Timelines

| Stage | Target |
| --- | --- |
| Document check | 5 business days from complete application |
| Interview scheduled | 10 business days |
| Decision | 5 business days after interview |
| Enrollment | 2 business days after decision |
| Periodic review | Every 12 months; overdue review triggers `lapsed_review` suspension after a 30-day grace period |

## 4. Appeals and suspension review

- Every rejection or suspension notice states the reason code and appeal rights.
- Appeal window: 14 days from notice, via the [appeal form](templates/appeal-form.md).
- An appeal is decided by a reviewer who did not take the original decision,
  within 15 business days. The outcome is final within this process.
- Suspension review: a suspension for `fraud_flag` is reviewed within 10
  business days. Outcome is `reinstate_attester` (with record) or
  `remove_attester`. Emergency suspension does not wait for review.
- Every outcome is added to the evidence bundle and the bundle hash is
  recomputed; the new hash is applied with `update_attester_info`.

## 5. Mapping to runbooks

| Event | Runbook |
| --- | --- |
| Fraud flag on an enrolled attester | [Attester fraud response](../runbooks/attester-fraud-response.md) |
| Key loss or compromise | [CHW key custody](../security/chw-key-custody.md), section 3 |
| Registrar or admin change | [Contract upgrade](../runbooks/contract-upgrade.md) and registry admin transfer procedures |

## Review status

- [ ] Reviewed and approved by the governance/legal owner (name, date): ______

This document is a proposal until the box above is completed.
