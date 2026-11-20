# Kenya Data Protection Act 2019 compliance analysis of on-chain data

> **Status:** Draft for data-protection review. **Scope:** the contract layer
> only (`attester-registry`, `attestation-registry`, `multisig-account`) as of
> this commit. **Reviewer (DPO / data-protection expert):** _to be recorded in
> the PR that merges this document._
>
> This document follows the structure of
> [the NDPA analysis](ndpa-onchain-analysis.md). The on-chain data inventory
> (NDPA §3.1–3.3) is identical and is not repeated; what changes is the legal
> basis, the rights regime, the registration and transfer rules, and the
> regulator.

## 1. Why this analysis exists

Kenya's Data Protection Act 2019 (DPA 2019) is administered by the **Office of
the Data Protection Commissioner (ODPC)**. It applies to a data controller or
processor **established in Kenya**, and to one not established in Kenya that
processes personal data of data subjects **located in Kenya** (s.4). A
deployment serving Kenyan CHWs and patients is in scope regardless of where the
operating organisation is registered.

"Personal data" means any information relating to an identified or
**identifiable** natural person (s.2). Pseudonymous ledger data is in scope
where it can be linked back, which is the same conclusion as the NDPA analysis.

**Health data is sensitive personal data** under s.2 (data revealing health
status). This is the key difference from Ghana: Kenya has an explicit special
category, and s.46 imposes extra conditions on it.

## 2. Relationship to `lafiya-docs`

As for NDPA: `lafiya-docs` owns the system-wide privacy design, off-chain
storage, consent flows and controller/processor roles. This document covers
only what the contracts write to the ledger.

## 3. Data inventory and lawful basis

The **Personal data** classification is identical to
[NDPA §3](ndpa-onchain-analysis.md#3-data-inventory). The lawful-basis column
below replaces the NDPA column.

### 3.1 Lawful basis mapping (DPA 2019 s.30, s.46)

s.30 permits processing where the data subject consents, or where it is
necessary for a contract, a legal obligation, vital interest, public interest,
or the legitimate interests of the controller or a third party.

| On-chain element (see NDPA §3) | Data subject | Expected basis under DPA 2019 | Notes |
| --- | --- | --- | --- |
| `DataKey::Admin`, `PendingAdmin`, admin events | Admin operator | Legitimate interest (s.30(f)) | Organisational multisigs only (NDPA R8). |
| `DataKey::Attester(Address)` → `AttesterInfo` | CHW | Contract (s.30(b)) or legitimate interest | The CHW is engaged by the programme. |
| `AttesterInfo.license_hash` | CHW | Legal obligation (s.30(c)) for licensure verification | Salted hash required (NDPA R3). |
| `AttesterInfo.region` | CHW | Legitimate interest (s.30(f)) | Coarsen (NDPA R2). |
| `Suspended` state and suspension events | CHW | Legitimate interest — patient safety | No reason codes on-chain (NDPA R1). |
| `Attestation(record_hash, seq)`, `AttestationRecorded` | Patient, CHW | **Sensitive personal data: s.46(a) explicit consent**, or s.46(b) medical diagnosis / provision of health care by a health professional bound by confidentiality | The CHW is the health professional; the controller must document the s.46(b) reliance or obtain explicit consent. |
| `AttestationRevoked` | Patient | As above | Record the request off-chain. |
| `Signer`, `Threshold`, `SignerCount` | Signers | Legitimate interest | Organisational keys only. |

**s.46 is the binding constraint.** Health data may only be processed under
s.46(a) explicit consent or s.46(b) where processing is necessary for a medical
purpose by a health professional under a duty of confidentiality. A deployment
that cannot show one of these must not write patient-linked elements.

## 4. Data-subject rights and erasure strategy

DPA 2019 s.26–s.40 give rights to be informed, to access, to correction, to
erasure (s.40), to object, and to data portability. The contract-layer strategy
is the same as NDPA §4:

| Element | Strategy | Sufficient under DPA 2019? |
| --- | --- | --- |
| `record_hash` (storage and events) | Crypto-shredding: destroy the off-chain salt and record, then `revoke_attestation`. | **Yes, if salts are mandatory** (NDPA R4). |
| `{attester, timestamp}` linked to `record_hash` | Falls with the hash. | Yes once the hash is shredded. |
| Attester address, `region`, `license_hash`, lifecycle events | `remove_attester`; destroy the off-chain identity mapping and the `license_hash` salt. | **Partial** — public history stays linkable to prior holders of the mapping. |
| `Suspended` state and events | Reinstate or remove. | **Partial** — no reason codes on-chain (NDPA R1). |
| Admin/signer addresses | Rotate keys; organisational multisigs. | Yes if operators are organisations. |

Rectification (s.38) is by `revoke_attestation` plus re-attestation over the
corrected, re-salted record.

## 5. Kenya-specific obligations

| Obligation | Provision | Contract-layer impact |
| --- | --- | --- |
| **Registration of data controllers/processors** | s.18 | Controllers and processors must register with the ODPC. Deployment prerequisite; no contract change. |
| **Data protection impact assessment** | s.31 | Required where processing is likely to result in high risk, which sensitive health data on a public ledger is. This document plus NDPA §5 is the contract-layer input to the DPIA. |
| **Data protection by design / default** | s.41 | Supports the hash-only footprint and the mandatory-salt recommendation (NDPA R4). |
| **Cross-border transfer** | s.48 | Transfer out of Kenya requires proof of appropriate safeguards, an adequacy decision, or a s.48(1) derogation (consent, contract, vital interest). A public ledger is a transfer to every jurisdiction — see §7 Q1. |
| **Breach notification** | s.43 | Off-chain; the contracts emit no breach signal. |
| **Offence for unlawful disclosure** | s.72 | Unlawful disclosure of personal data is an offence. This raises the stakes on the CHW re-identification risk (NDPA K2). |

## 6. Risk scoring (contract layer)

Same method as NDPA §5. Only Kenya-specific deltas are listed; K1–K7 carry
over unchanged.

| # | Risk | L | I | Risk | Residual |
| --- | --- | --- | --- | --- | --- |
| KE1 | Sensitive health data written to a public ledger without a documented s.46 basis | 3 | 3 | **9 high** | 3 low (s.46 basis recorded per deployment) |
| KE2 | Cross-border transfer under s.48 without appropriate safeguards | 3 | 3 | **9 high** | 4 medium (Q1 resolved by counsel) |
| KE3 | No DPIA before mainnet deployment (s.31) | 2 | 2 | 4 medium | 1 low (deployment checklist) |
| KE4 | Unlawful-disclosure offence (s.72) triggered by CHW re-identification | 2 | 3 | **6 medium** | 4 medium (NDPA R2, R3) |

## 7. Open legal questions for counsel

- **Q1 (s.48).** Does a salted commitment on a public ledger constitute a
  cross-border transfer, and which s.48 ground applies? Does crypto-shredding
  change the answer?
- **Q2 (s.46(b)).** Is a CHW attesting a record "provision of health care by a
  health professional" for s.46(b), or must the controller obtain explicit
  consent under s.46(a)?
- **Q3 (s.40).** Is destroying the salt an adequate response to an erasure
  request when the on-chain hash remains?
- **Q4 (s.18).** Must each deploying organisation register separately with the
  ODPC?
- **Q5 (s.31).** Does the ODPC require a DPIA per deployment or per programme?

## 8. Recommendations

The NDPA recommendations (R1–R8) apply, plus:

| ID | Recommendation | Addresses |
| --- | --- | --- |
| KE-R1 | Record the s.46 basis (explicit consent or health-professional reliance) per deployment before any Kenya deployment. | KE1 |
| KE-R2 | Record the s.48 transfer ground in the deployment checklist. | KE2 |
| KE-R3 | Add ODPC registration and a DPIA to the deployment prerequisites in `deployments/README.md`. | KE3 |
