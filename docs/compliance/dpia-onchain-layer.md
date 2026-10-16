# DPIA: Lafiya on-chain layer, indexer and dashboard

Uses [dpia-template.md](dpia-template.md). Builds on [ndpa-onchain-analysis.md](ndpa-onchain-analysis.md) (data inventory, risks K1–K7, recommendations R1–R8) and [linkability.md](../security/linkability.md).

## 1. Document control

| Field | Value |
| --- | --- |
| Scope | Soroban contracts (`attester-registry`, `attestation-registry`, `multisig-account`, `incentive-pool`); `lafiya-indexer`; transparency dashboard (`dashboard/`, fed by `lafiya-watchdog`) |
| Version | 1.0 draft, 2026-10 |
| Assessor | Engineering (repository maintainers) |
| Roles | Deploying programme = controller; Lafiya maintainers = software provider (a processor only if they run a production indexer); Stellar validators and RPC providers = public infrastructure |
| Status | **Draft: awaiting DPO and counsel review (see §5)** |

## 2. Description of processing

| Component | Data | Subjects | Purpose | Retention |
| --- | --- | --- | --- | --- |
| Contracts | Attester address, `license_hash`, region, enrolment/suspension state; `record_hash` + attester + timestamp per attestation; signer keys; pool payouts per attester | CHWs, patients (pseudonymous), operators | Verify that a CHW-attested record exists and pay incentives | Storage until TTL/removal; **events and ledger history permanent** |
| Indexer | Decoded contract events, ledger cursor, Postgres rows keyed by `record_hash`/attester | As above | Query API, reconciliation | Currently unbounded (D5) |
| Dashboard | Aggregates of verification activity and incentive accounting from the event export (`events.jsonl` on `gh-pages`) | Aggregated; small cells may identify | Public transparency | Append-only; public |

No plaintext health data is on-chain ([ADR-0001](../adr/0001-hash-only-on-chain-footprint.md)). The off-chain record, salt, and patient card are outside this DPIA's boundary but decide whether `record_hash` is personal data (it is, if the committed fields are low-entropy and unsalted).

## 3. Necessity and proportionality

- **Lawful basis:** set by the controller; expected bases per element are in ndpa-onchain-analysis.md §3 (contract/legitimate interest for CHWs; vital interest/consent for patients).
- **Minimisation:** hash-only footprint; no names, diagnoses or free-text reasons (R1). Residual identifiers: CHW address/region/licence hash and patient `record_hash`.
- **Erasure (s.34):** ledger history cannot be deleted; the strategy is crypto-shredding of the off-chain record and salt plus `revoke_attestation` (R4, R6).
- **Transparency:** the public dashboard should publish aggregates only, with small-cell suppression (D6).
- **Processors:** RPC providers and any hosted indexer see queries and are processors if the controller engages them.

## 4. Risk register

Scores follow the template. `K*` IDs come from the contract-layer analysis; `D*` IDs are added here for the off-chain components.

| ID | Risk | L | I | Score | Mitigation | Residual | Tracking |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K1 | Dictionary recovery of patient data from unsalted `record_hash` | 3 | 3 | 9 high | Mandatory ≥128-bit salt in LRC-1 (R4) | 2 low | Analysis [#432](https://github.com/Lafiya-xyz/Lafiya-contract/issues/432) (closed). Schema-enforcement issue: **not yet filed** |
| K2 | CHW re-identification via address + region + enrolment time | 3 | 2 | 6 medium | Coarse region (R2); onboarding workflow | 4 medium | [#614](https://github.com/Lafiya-xyz/Lafiya-contract/issues/614). R2 issue: **not yet filed** |
| K3 | Reversal of unsalted `license_hash` | 3 | 2 | 6 medium | Salted/HMAC licence hash (R3) | 2 low | [#614](https://github.com/Lafiya-xyz/Lafiya-contract/issues/614). R3 issue: **not yet filed** |
| K4 | Disciplinary status exposed via suspension state/events | 2 | 2 | 4 medium | No reason codes on-chain (R1) | 4 medium | Accepted risk (patient safety). Owner: controller DPO |
| K5 | Visit patterns from re-attestation frequency and timestamps | 2 | 2 | 4 medium | Blinded commitments (ADR-0012); coarse timestamps | 2 low | [#433](https://github.com/Lafiya-xyz/Lafiya-contract/issues/433) (closed) and its LRC-2 follow-ups |
| K6 | Erasure impossible on a permanent ledger | 3 | 2 | 6 medium | Crypto-shredding runbook (R6) | 2 low | Erasure runbook issue: **not yet filed** |
| K7 | Future fields leak patient data | 2 | 3 | 6 medium | CONTRIBUTING on-chain data checklist; schema-impact CI; this DPIA's triggers | 2 low | Process control (in place) |
| D1 | Source-account/device linkage of attestations | 2 | 2 | 4 medium | Relayed, sponsored submission | 2 low | [#611](https://github.com/Lafiya-xyz/Lafiya-contract/issues/611) |
| D2 | Fraud scoring over indexed data profiles CHWs | 2 | 2 | 4 medium | Score pseudonymous activity only; human review before action; no automated sanction | 2 low | #415 (closed); review step in [attester-fraud-response](../runbooks/attester-fraud-response.md) |
| D5 | Indexer retains all decoded fields indefinitely; a hosted indexer becomes a processor holding linkable data | 3 | 2 | 6 medium | Retention policy, pruning, field review | 3 low | [#522](https://github.com/Lafiya-xyz/Lafiya-contract/issues/522) |
| D6 | Dashboard small cells (low-volume region/period) single out a CHW or patient | 2 | 3 | 6 medium | k-anonymity / small-cell suppression in the pipeline | 2 low | [#577](https://github.com/Lafiya-xyz/Lafiya-contract/issues/577) |
| D7 | Raw per-event export (addresses, hashes) republished permanently on `gh-pages` | 2 | 3 | 6 medium | Publish aggregates only; keep raw export out of the public branch | 2 low | **Not yet filed**; verify what `dashboard.yml` publishes before launch |

**Tracking gaps:** rows marked *not yet filed* have no GitHub issue today. Maintainers must file them (or record an accepted-risk decision in the row) before this DPIA moves from Draft to Approved.

## 5. Consultation and sign-off

| Step | Who | Date | Outcome |
| --- | --- | --- | --- |
| Technical review | _pending_ | | |
| DPO review | _pending_ | | |
| Counsel review (outcome category only; no confidential content) | _pending_ | | _not yet performed_ |
| Approver | _pending_ | | |

Nothing here has been reviewed by counsel or a DPO yet; a human reviewer must complete this table.

## 6. Review and update triggers

See the [template §6](dpia-template.md#6-review-and-update-triggers). This DPIA is linked from [ndpa-onchain-analysis.md](ndpa-onchain-analysis.md) and the [release checklist](../releasing.md#data-protection-review).
