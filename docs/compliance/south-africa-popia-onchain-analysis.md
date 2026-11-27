# South Africa POPIA compliance analysis of on-chain data

> **Status:** Draft for data-protection review. **Scope:** the contract layer
> only (`attester-registry`, `attestation-registry`, `multisig-account`) as of
> this commit. **Reviewer (DPO / data-protection expert):** _to be recorded in
> the PR that merges this document._
>
> This document follows the structure of
> [the NDPA analysis](ndpa-onchain-analysis.md). The on-chain data inventory
> (NDPA §3.1–3.3) is identical and is not repeated; what changes is the legal
> basis, the rights regime, the cross-border rules, and the regulator.

## 1. Why this analysis exists

The Protection of Personal Information Act 4 of 2013 (POPIA) is administered
by the **Information Regulator (South Africa)**. It applies to a responsible
party domiciled in South Africa, and to one not domiciled there that makes use
of processing means in South Africa (s.3(1)(b)). A deployment serving South
African CHWs and patients is in scope.

"Personal information" includes information relating to an identifiable, living
natural person, and explicitly includes **biometrics** and **information about
the health of a person** (s.1). Health information is therefore personal
information *and* is subject to the additional s.32 condition. Pseudonymous
ledger data is in scope where it can be linked back, as for NDPA.

## 2. Relationship to `lafiya-docs`

As for NDPA: `lafiya-docs` owns the system-wide privacy design, off-chain
storage, consent flows and responsible-party/operator roles. This document
covers only what the contracts write to the ledger.

## 3. Data inventory and lawful basis

The **Personal data** classification is identical to
[NDPA §3](ndpa-onchain-analysis.md#3-data-inventory). POPIA uses
"responsible party" for controller and "operator" for processor.

### 3.1 Lawful basis mapping (POPIA s.11, s.27, s.32)

s.11 permits processing where the data subject consents, or where it is
necessary for a contract, a legal obligation, protection of a legitimate
interest, performance of a public-law duty, or protection of a legitimate
interest of the data subject.

| On-chain element (see NDPA §3) | Data subject | Expected basis under POPIA | Notes |
| --- | --- | --- | --- |
| `DataKey::Admin`, `PendingAdmin`, admin events | Admin operator | Legitimate interest (s.11(1)(f)) | Organisational multisigs only (NDPA R8). |
| `DataKey::Attester(Address)` → `AttesterInfo` | CHW | Contract (s.11(1)(b)) or legitimate interest | The CHW is engaged by the programme. |
| `AttesterInfo.license_hash` | CHW | Legal obligation (s.11(1)(c)) for licensure verification | Salted hash required (NDPA R3). |
| `AttesterInfo.region` | CHW | Legitimate interest | Coarsen (NDPA R2). |
| `Suspended` state and suspension events | CHW | Legitimate interest — patient safety | No reason codes on-chain (NDPA R1). |
| `Attestation(record_hash, seq)`, `AttestationRecorded` | Patient, CHW | **s.27(1)(a) consent** or **s.27(1)(d) medical purpose** by a health care professional | s.32 adds conditions: the information must have been obtained from the data subject or a health professional, and the processing must be necessary for a medical purpose. |
| `AttestationRevoked` | Patient | As above | Record the request off-chain. |
| `Signer`, `Threshold`, `SignerCount` | Signers | Legitimate interest | Organisational keys only. |

**s.32 is the binding constraint.** Health information may only be processed by
a health care professional, a health care institution, or a person bound by a
confidentiality duty, and only for a medical purpose or related purposes. The
CHW must be able to show that status; otherwise the responsible party must rely
on s.27(1)(a) consent.

## 4. Data-subject rights and erasure strategy

POPIA s.5, s.11(3), s.23–s.25 give rights to be notified, to access, to
correction and deletion, and to object. The contract-layer strategy is the
same as NDPA §4:

| Element | Strategy | Sufficient under POPIA? |
| --- | --- | --- |
| `record_hash` (storage and events) | Crypto-shredding: destroy the off-chain salt and record, then `revoke_attestation`. | **Yes, if salts are mandatory** (NDPA R4). |
| `{attester, timestamp}` linked to `record_hash` | Falls with the hash. | Yes once the hash is shredded. |
| Attester address, `region`, `license_hash`, lifecycle events | `remove_attester`; destroy the off-chain identity mapping and the `license_hash` salt. | **Partial** — public history stays linkable to prior holders of the mapping. |
| `Suspended` state and events | Reinstate or remove. | **Partial** — no reason codes on-chain (NDPA R1). |
| Admin/signer addresses | Rotate keys; organisational multisigs. | Yes if operators are organisations. |

Correction (s.24) is by `revoke_attestation` plus re-attestation over the
corrected, re-salted record.

## 5. South Africa-specific obligations

| Obligation | Provision | Contract-layer impact |
| --- | --- | --- |
| **Information officer registration** | s.55 | The responsible party must register its information officer with the Regulator. Deployment prerequisite; no contract change. |
| **Prior authorisation** | s.57, s.58 | Prior authorisation is required for processing of health information **by bodies other than health care professionals** (s.57(1)(a)), and for transfers under s.57(1)(d). If the deploying organisation is not itself a health care professional, this is a hard gate. |
| **Security safeguards** | s.19 | Reasonable technical and organisational measures. The off-chain salt store is the control that matters; on-chain immutability is not a safeguard. |
| **Cross-border transfer** | s.72 | Transfer out of South Africa requires one of the s.72(1) grounds: consent, contract necessity, a benefit to the data subject, or a transfer to a jurisdiction with adequate protection. A public ledger is a transfer to every jurisdiction — see §7 Q1. |
| **Breach notification** | s.22 | Off-chain; the contracts emit no breach signal. |
| **Direct marketing / automated decisions** | s.69, s.71 | Not applicable to the contract layer today; revisit if scoring is added. |

## 6. Risk scoring (contract layer)

Same method as NDPA §5. Only South Africa-specific deltas are listed; K1–K7
carry over unchanged.

| # | Risk | L | I | Risk | Residual |
| --- | --- | --- | --- | --- | --- |
| ZA1 | Health information processed without a s.32 basis | 3 | 3 | **9 high** | 3 low (s.32 basis recorded per deployment) |
| ZA2 | Prior authorisation (s.57) not obtained before processing health information | 2 | 3 | **6 medium** | 2 low (deployment checklist) |
| ZA3 | Cross-border transfer under s.72 without a ground | 3 | 3 | **9 high** | 4 medium (Q1 resolved by counsel) |
| ZA4 | Information officer not registered (s.55) | 2 | 2 | 4 medium | 1 low (deployment checklist) |

## 7. Open legal questions for counsel

- **Q1 (s.72).** Does a salted commitment on a public ledger constitute a
  cross-border transfer, and which s.72(1) ground applies? Does crypto-shredding
  change the answer?
- **Q2 (s.32).** Is a CHW a "health care professional" for s.32, or must the
  responsible party rely on consent?
- **Q3 (s.57(1)(a)).** Does the deploying organisation need prior authorisation
  because it processes health information and is not itself a health care
  professional?
- **Q4 (s.24).** Is destroying the salt an adequate response to a deletion
  request when the on-chain hash remains?
- **Q5 (s.55).** Must each deploying organisation register its own information
  officer?

## 8. Recommendations

The NDPA recommendations (R1–R8) apply, plus:

| ID | Recommendation | Addresses |
| --- | --- | --- |
| ZA-R1 | Record the s.32 basis (health care professional status or consent) per deployment before any South Africa deployment. | ZA1 |
| ZA-R2 | Add prior-authorisation screening (s.57) to the deployment checklist. | ZA2 |
| ZA-R3 | Record the s.72 transfer ground in the deployment checklist. | ZA3 |
| ZA-R4 | Add information-officer registration (s.55) to the deployment prerequisites in `deployments/README.md`. | ZA4 |
