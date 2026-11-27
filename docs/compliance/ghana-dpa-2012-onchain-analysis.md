# Ghana Data Protection Act 2012 (Act 843) compliance analysis of on-chain data

> **Status:** Draft for data-protection review. **Scope:** the contract layer
> only (`attester-registry`, `attestation-registry`, `multisig-account`) as of
> this commit. **Reviewer (DPO / data-protection expert):** _to be recorded in
> the PR that merges this document._
>
> This document follows the structure of
> [the NDPA analysis](ndpa-onchain-analysis.md). It does not repeat the data
> inventory: the on-chain elements are identical, so sections 3.1–3.3 of the
> NDPA analysis apply verbatim. What changes per jurisdiction is the legal
> basis, the data-subject rights, the registration/transfer rules, and the
> regulator.

## 1. Why this analysis exists

Ghana's Data Protection Act 2012 (Act 843) is administered by the **Data
Protection Commission (DPC)**. It applies to a controller or processor who is
established in Ghana, or who is not established in Ghana but uses equipment in
Ghana to process personal data other than for transit (s.3). A deployment
serving Ghanaian CHWs and patients is therefore in scope even if the operating
organisation is registered elsewhere.

Act 843 defines "personal data" as data about an individual who can be
identified from the data or from the data **in combination with other
information in the possession of, or likely to come into the possession of,
the data controller** (s.96). Pseudonymous ledger data (addresses, hashes,
timestamps) is in scope whenever the controller or a third party can link it
back, which is the same conclusion the NDPA analysis reaches for Nigeria.

## 2. Relationship to `lafiya-docs`

As for NDPA: `lafiya-docs` owns the system-wide privacy design, off-chain
storage, consent flows and controller/processor roles. This document covers
only what the contracts write to the ledger and defers to `lafiya-docs` for
lawful-basis selection, off-chain retention, consent text and request handling.

## 3. Data inventory

Identical to [NDPA §3](ndpa-onchain-analysis.md#3-data-inventory). The
**Personal data** classification is jurisdiction-independent: an element that
is personal data under NDPA is personal data under Act 843. The lawful-basis
column below replaces the NDPA column.

### 3.1 Lawful basis mapping (Act 843 s.17)

Act 843 permits processing only if at least one condition in s.17 is met. The
conditions are: consent; contract; legal obligation; vital interest; public
interest; and legitimate interest of the controller or a third party, where
that interest is not overridden by the data subject's rights.

| On-chain element (see NDPA §3) | Data subject | Expected basis under Act 843 | Notes |
| --- | --- | --- | --- |
| `DataKey::Admin`, `PendingAdmin`, admin events | Admin operator | Legitimate interest (s.17(f)) | Prefer organisational multisigs (NDPA R8). |
| `DataKey::Attester(Address)` → `AttesterInfo` | CHW | Contract (s.17(b)) with the CHW, or legitimate interest of the programme | The CHW is engaged by the programme; the enrolment record is the contract evidence. |
| `AttesterInfo.license_hash` | CHW | Legal obligation (s.17(c)) for licensure verification | Must be salted (NDPA R3); an unsalted hash of a licence number is reversible. |
| `AttesterInfo.region` | CHW | Legitimate interest (s.17(f)) | Coarsen to region/zone (NDPA R2). |
| `Suspended` state and suspension events | CHW | Legitimate interest — patient safety (s.17(f)) | No reason codes on-chain (NDPA R1). |
| `Attestation(record_hash, seq)`, `AttestationRecorded` | Patient, CHW | **Consent (s.17(a))** or vital interest (s.17(d)) | Consent must be "freely given, specific and informed" (s.18); the controller records it off-chain. Vital interest is only available where the patient cannot consent. |
| `AttestationRevoked` | Patient | As above | Revocation is itself a data-subject action; record it off-chain. |
| `Signer`, `Threshold`, `SignerCount` | Signers | Legitimate interest (s.17(f)) | Organisational keys only. |

**Health data.** Act 843 does not create a separate "special category" the way
GDPR Art. 9 does, but s.18 requires that consent be explicit where the
processing concerns health data, and the DPC's guidance treats health data as
sensitive. Treat every patient-linked element as requiring explicit consent or
a documented vital-interest justification.

## 4. Data-subject rights and erasure strategy

Act 843 s.24 gives the data subject the right to access, and s.32–s.34 give
rights to correction and, where the processing is unlawful or no longer
necessary, to blocking, erasure or destruction of the data. The contract-layer
strategy is the same as NDPA §4:

| Element | Strategy | Sufficient under Act 843? |
| --- | --- | --- |
| `record_hash` (storage and events) | Crypto-shredding: destroy the off-chain salt and record, then `revoke_attestation`. | **Yes, if salts are mandatory** (NDPA R4). Without a salt the hash remains confirmable by enumeration. |
| `{attester, timestamp}` linked to `record_hash` | Falls with the hash. | Yes once the hash is shredded. |
| Attester address, `region`, `license_hash`, lifecycle events | `remove_attester` stops processing; destroy the off-chain identity mapping and the salt for `license_hash`. | **Partial** — public history stays linkable to anyone who already knew the mapping. |
| `Suspended` state and events | Reinstate or remove. | **Partial** — reason codes must never go on-chain (NDPA R1). |
| Admin/signer addresses | Rotate keys; use organisational multisigs. | Yes if operators are organisations. |

Rectification (s.32) is by `revoke_attestation` plus re-attestation over the
corrected, re-salted record. Nothing is edited in place.

## 5. Ghana-specific obligations

| Obligation | Provision | Contract-layer impact |
| --- | --- | --- |
| **Registration of data controllers** | s.27–s.29 | The deploying controller must register with the DPC before processing. No contract change; a deployment prerequisite recorded in `deployments/`. |
| **Security safeguards** | s.28 | The controller must take reasonable technical measures. On-chain immutability is a design constraint, not a safeguard; the off-chain salt store is the control that matters. |
| **Data-processing records** | s.30 | The controller keeps a record of processing. The data inventory in NDPA §3 and §7 is the contract-layer part of that record. |
| **Cross-border transfer** | s.36 | Personal data may leave Ghana only with consent, for contract performance, or where the destination has adequate protection. A public ledger is a transfer to every jurisdiction. This is the single largest Ghana-specific gap — see §7 Q1. |
| **Data subject access** | s.24 | Everything on-chain is public; the controller answers access requests from its off-chain mapping. |
| **Notification of the DPC** | s.31 | Breach notification is off-chain; the contracts emit no breach signal. |

## 6. Risk scoring (contract layer)

Same method as NDPA §5 (L × I, 1–3 each). Only the Ghana-specific deltas are
listed; K1–K7 carry over unchanged.

| # | Risk | L | I | Risk | Residual |
| --- | --- | --- | --- | --- | --- |
| G1 | Cross-border transfer of patient-linked data to a public ledger without an s.36 basis | 3 | 3 | **9 high** | 4 medium (Q1 resolved by counsel; consent wording covers transfer) |
| G2 | Controller not registered with the DPC before mainnet deployment | 2 | 2 | 4 medium | 1 low (deployment checklist) |
| G3 | Explicit-consent requirement for health data not evidenced off-chain | 2 | 3 | **6 medium** | 2 low (`lafiya-docs` consent flow) |

## 7. Open legal questions for counsel

- **Q1 (s.36).** Does writing a salted commitment to a public ledger constitute
  a cross-border transfer of personal data, and if so which s.36 ground does
  the controller rely on? Does crypto-shredding change the answer?
- **Q2 (s.17(a)/s.18).** Is consent "freely given" for a patient who must be
  attested to receive care, or must the controller rely on vital interest?
- **Q3 (s.24/s.32).** Is destroying the salt an adequate response to an access
  or correction request when the on-chain hash remains?
- **Q4 (s.27).** Must each deploying organisation register separately, or can
  the software maintainer register once for all deployments?
- **Q5.** Does the DPC treat a CHW's professional licence hash as health data
  or as ordinary personal data?

## 8. Recommendations

The recommendations are the NDPA ones (R1–R8) plus the Ghana-specific items
below. Tracking is recorded in
[the cross-jurisdiction matrix](multi-jurisdiction-matrix.md#5-design-constraints).

| ID | Recommendation | Addresses |
| --- | --- | --- |
| G-R1 | Record the s.36 transfer basis in the deployment checklist before any Ghana deployment. | G1 |
| G-R2 | Add DPC registration to the deployment prerequisites in `deployments/README.md`. | G2 |
| G-R3 | Evidence explicit consent for health data in the `lafiya-docs` consent flow. | G3 |
