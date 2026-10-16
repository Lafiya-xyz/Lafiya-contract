# DPIA template (NDPA 2023)

Copy this file to `docs/compliance/dpia-<system>-<YYYY-MM>.md`, fill every section, and have counsel review it before release. The completed assessment for the on-chain layer is [dpia-onchain-layer.md](dpia-onchain-layer.md).

**Do not put confidential legal advice, patient data, or secrets in a DPIA.** Record *that* counsel reviewed it and the outcome category, not the content of their advice.

## 1. Document control

| Field | Value |
| --- | --- |
| System / scope | |
| Version / date | |
| Assessor(s) | |
| Controller / processor roles | |
| Status | Draft / In review / Approved / Superseded |

## 2. Description of processing

Nature, scope, context, purposes; data categories; data subjects; recipients; retention; international transfers; systems and data flows (link to the data inventory).

## 3. Necessity and proportionality

Lawful basis per processing activity (NDPA s.25); data minimisation; accuracy; storage limitation; data-subject rights and how they are honoured (including erasure on an append-only ledger); processor and sub-processor controls.

## 4. Risk assessment

Likelihood (L) and impact (I) are 1–3; **risk = L × I** (1–3 low, 4–6 medium, 7–9 high). Every risk gets a row in the register with a mitigation and a **tracking issue** (or an explicit accepted-risk decision with an owner).

| ID | Risk | L | I | Score | Mitigation | Residual | Tracking issue / decision |
| --- | --- | --- | --- | --- | --- | --- | --- |

## 5. Consultation and sign-off

| Step | Who | Date | Outcome |
| --- | --- | --- | --- |
| Technical review | | | |
| DPO / data-protection review | | | |
| **Counsel review** (record that it happened and the outcome category only: approved / approved with conditions / changes required) | | | |
| Approver (accountable owner) | | | |

## 6. Review and update triggers

Re-run (or amend and re-sign) the DPIA when any of these occur. The release checklist in [docs/releasing.md](../releasing.md#data-protection-review) links here.

- A `#[contracttype]` stored under a `DataKey`, or a `#[contractevent]`, is added or changed (the schema-impact CI check flags this).
- The commitment format (LRC-x), QR payload, or protocol/interface version changes.
- The indexer or dashboard stores a new field, changes retention, or changes aggregation thresholds.
- A new processor, relayer or data recipient is introduced.
- A privacy incident or a regulator or data-subject complaint.
- Otherwise, at least every 12 months.
