# NDPA 2023 compliance analysis of on-chain data

> **Status:** Draft for data-protection review. **Scope:** the contract layer
> only (`attester-registry`, `attestation-registry`, `multisig-account`) as of
> this commit. **Reviewer (DPO / data-protection expert):** _to be recorded in
> the PR that merges this document._

## 1. Why this analysis exists

The README says the Nigeria Data Protection Act 2023 (NDPA) governs the
project and that no health data is ever written on-chain
([ADR-0001](../adr/0001-hash-only-on-chain-footprint.md)). That holds for
*plaintext* health data. It does not settle the question, because under NDPA
s.65 "personal data" is any information relating to an identified or
**identifiable** natural person. Pseudonymous data (addresses, hashes,
timestamps) is still personal data if someone can link it back to a person
with reasonable effort, and a public ledger lets anyone try.

Two groups of data subjects are affected:

- **Community health workers (CHWs)**, who act as attesters.
- **Patients**, whose records are committed to by `record_hash`.

## 2. Relationship to `lafiya-docs`

[`lafiya-docs`](https://github.com/Lafiya-xyz/lafiya-docs) owns the
system-wide **privacy design** and **threat model**: what the web app
collects, off-chain storage, consent flows, and the controller/processor
roles of the deploying organisation. This document does **not** repeat that.
It covers only what the contracts in this repository write to the ledger
(storage and events), and it defers to `lafiya-docs` for:

- the lawful basis actually chosen by each deploying controller;
- off-chain retention of records, salts and CHW identity mappings;
- consent text and data-subject request handling procedures.

If the two documents disagree, `lafiya-docs` states the policy and this
document must be updated to show how the contracts satisfy it.

## 3. Data inventory

Legend for **Personal data**: *Yes* (identifies or singles out a person on
its own or with commonly available data), *Maybe* (identifies with auxiliary
off-chain data or through correlation), *No*.

Lawful basis refers to the grounds in NDPA s.25. It is set by the deploying controller;
the column gives the expected basis.

Retention: persistent Soroban storage lives until its TTL expires unless it
is extended, and it can be archived and restored. **Events and transaction
history are permanent** in ledger history and in any third-party indexer.

### 3.1 `attester-registry`

| Element | Kind | Personal data | Data subject | Why | Expected lawful basis | Retention |
| --- | --- | --- | --- | --- | --- | --- |
| `DataKey::Admin`, `PendingAdmin` | storage | Maybe | Admin operator | An address; identifies an individual only if the admin is a person, not a multisig or org | Legitimate interest | Until rotated; history permanent |
| `DataKey::Attester(Address)` → `AttesterInfo` | storage | **Yes** | CHW | The CHW's address, allowlisted by a known programme. The enrolling organisation knows the mapping, and small cohorts make it guessable | Contract / legitimate interest (employment of CHW) | Until `remove_attester`; history permanent |
| `AttesterInfo.license_hash` | storage | **Yes** | CHW | A hash of a professional licence number. Licence numbers have low entropy and appear in public registers, so an unsalted hash can be reversed by enumeration | Legal obligation for licensure checks | As above |
| `AttesterInfo.region` | storage | **Yes** (with address) | CHW | Region plus enrolment time narrows a small LGA cohort to one person | Legitimate interest | As above |
| `DataKey::Suspended(Address)` | storage | **Yes** | CHW | Reveals an employment/disciplinary state of an identifiable worker | Legitimate interest (patient safety) | Until reinstated; history permanent |
| `DataKey::AttesterCount`, `MaxAttesters`, `Paused`, `SchemaVersion` | storage | No | — | Aggregate or configuration values | — | — |
| `AttesterAdded`, `AttesterRemoved`, `AttesterInfoUpdated` | event | **Yes** | CHW | Timestamped enrolment history of an identifiable worker | Legitimate interest | **Permanent** |
| `AttesterSuspended`, `AttesterReinstated` | event | **Yes** | CHW | Timestamped disciplinary history. No reason code today; adding one would make it more sensitive | Legitimate interest | **Permanent** |
| `Initialized`, `AdminTransferred`, `Paused`, `Unpaused` | event | Maybe | Admin operator | As for `Admin` | Legitimate interest | **Permanent** |
| `Upgraded` | event | No | — | A wasm hash | — | Permanent |

### 3.2 `attestation-registry`

| Element | Kind | Personal data | Data subject | Why | Expected lawful basis | Retention |
| --- | --- | --- | --- | --- | --- | --- |
| `DataKey::Attestation(record_hash, seq)` → `{attester, timestamp}` | storage | **Yes** (patient: Maybe → Yes if unsalted) | Patient, CHW | The commitment refers to one patient's record. It can be recovered if the committed fields have low entropy ([ADR-0008 threat analysis](../adr/0008-record-commitment-canonicalization.md)). The timestamp and attester also place a patient with a CHW in a region at a time | Vital interest / consent, per controller | Bounded history per hash; ledger history permanent |
| `AttestationSequence`, `AttestationCount` (per hash) | storage | Maybe | Patient | Re-attestation frequency per record says how often a patient was seen | As above | As above |
| `AttestationRecorded {record_hash, attester, timestamp}` | event | **Yes** | Patient, CHW | Same linkage as storage, but permanent and trivially indexed | As above | **Permanent** |
| `AttestationRevoked {record_hash}` | event | Maybe | Patient | Signals that a specific record was retracted | As above | **Permanent** |
| `AttesterRegistryRepointed`, admin/pause events | event | Maybe | Admin operator | As in 3.1 | Legitimate interest | Permanent |

### 3.3 `multisig-account`

| Element | Kind | Personal data | Data subject | Why | Expected lawful basis | Retention |
| --- | --- | --- | --- | --- | --- | --- |
| `DataKey::Signer(BytesN<32>)`, `Threshold`, `SignerCount` | storage | Maybe | Signers | ed25519 keys of operators; personal only if a signer is an individual | Legitimate interest | Until the account is retired |

### 3.4 Not present today (future fields)

Supersession links, access logs, and card pointers do **not** exist in the
contracts. Each would be **Yes** for the patient: access logs reveal who
looked up a patient and when, and supersession links chain a patient's
records together. They must go through the checklist in
[CONTRIBUTING.md](../../CONTRIBUTING.md#on-chain-data-review-checklist) before
they are added.

## 4. Erasure strategy (NDPA data-subject rights, s.34)

Ledger data cannot be deleted. The strategy is to make the on-chain element
**unlinkable** to a person, so that what remains is no longer personal data,
and to stop further processing.

| Element | Strategy | Sufficient? |
| --- | --- | --- |
| `record_hash` (storage and events) | **Crypto-shredding.** Every LRC-1 commitment includes a random salt (≥128 bits) stored only off-chain. On an erasure request the controller destroys the salt and the record. The hash can then no longer be recomputed or confirmed from any guess. Also call `revoke_attestation` so no verifier treats it as live. | **Yes, if salts are mandatory.** Without a salt, destroying the off-chain record does not help: an attacker who guesses the fields can still confirm the hash. |
| `{attester, timestamp}` linked to `record_hash` | Falls with the hash: once the hash is unlinkable, "a CHW attested *something* at time T" is no longer about the patient. | Yes, once the hash is shredded. The CHW-side linkage remains (see below). |
| Attester address, `region`, `license_hash`, lifecycle events | `remove_attester` stops current processing. History cannot be erased. Destroy the off-chain address↔identity mapping held by the programme. Use **salted** `license_hash` so destroying the salt makes it unlinkable. | **Partial.** An address with public history stays linkable by anyone who already knew the mapping. Mitigate with per-deployment keys and minimal metadata (Recommendations R2 and R3). |
| `Suspended` state and events | Reinstate or remove. History cannot be erased. | **Partial.** This is why reason codes must never go on-chain (R1). |
| Admin/signer addresses | Rotate keys; use organisational multisigs rather than personal keys. | Yes, if operators are organisations. |

Rectification: a wrong attestation is corrected by
`revoke_attestation` and re-attestation over the corrected, re-salted record.
Nothing is edited in place.

Access: everything on-chain is already public. The controller's
answer to an access request lists the data subject's hashes and addresses
from its off-chain mapping.

## 5. DPIA-style risk scoring (contract layer)

Likelihood (L) and impact (I) are scored 1–3; risk = L × I (1–3 low, 4–6
medium, 7–9 high). "Residual" assumes the recommendations are implemented.

| # | Risk | L | I | Risk | Residual |
| --- | --- | --- | --- | --- | --- |
| K1 | Dictionary recovery of patient data from unsalted `record_hash` | 3 | 3 | **9 high** | 2 low (R4) |
| K2 | Re-identification of a CHW from address + region + enrolment timing in a small LGA | 3 | 2 | **6 medium** | 4 medium (R2, R3) |
| K3 | Reversal of unsalted `license_hash` against public licence registers | 3 | 2 | **6 medium** | 2 low (R3) |
| K4 | Disclosure of CHW disciplinary status via suspension state/events | 2 | 2 | 4 medium | 4 medium (accepted: needed for patient safety) — **9 high** if reason codes were added (R1) |
| K5 | Patient visit patterns inferred from re-attestation frequency and timestamps | 2 | 2 | 4 medium | 2 low (R4, R5) |
| K6 | Right to erasure cannot be honoured on a permanent ledger | 3 | 2 | **6 medium** | 2 low with crypto-shredding (R4, R6) |
| K7 | Future fields (access logs, supersession, card pointers) leak patient data | 2 | 3 | **6 medium** | 2 low (R7, checklist) |

## 6. Recommendations

Each recommendation needs a linked issue or an explicit accepted-risk
decision before this analysis is considered complete. The **Tracking**
column records that; maintainers fill in issue numbers as they file them.

| ID | Recommendation | Addresses | Tracking |
| --- | --- | --- | --- |
| R1 | Never put suspension/removal **reason codes** (or any free text) about a CHW on-chain; keep reasons off-chain with the programme. | K4 | Engineering issue to file: "Document and test: suspension events carry no reason". |
| R2 | Make `region` coarse (state or zone, not LGA/ward) or drop it from on-chain `AttesterInfo`; resolve fine-grained region off-chain. | K2 | Engineering issue to file. |
| R3 | Require `license_hash` to be a **salted** hash (e.g. HMAC with a per-programme secret) and document the construction; reject the raw SHA-256 of a licence number. | K3, K6 | Engineering issue to file. |
| R4 | Make a ≥128-bit random salt a **mandatory** LRC-1 field, so every `record_hash` can be crypto-shredded (closes the open question in ADR-0008). | K1, K5, K6 | Engineering issue to file (LRC-1 schema + `lafiya-web`). |
| R5 | Avoid per-patient re-attestation counters in events; if frequency is needed, expose it only through storage reads. | K5 | Accepted risk (proposed): the bounded history is needed for verification; revisit if analytics indexers appear. |
| R6 | Document the erasure runbook: destroy salt + record off-chain, call `revoke_attestation`, record the request. Add it to `docs/runbooks/`. | K6 | Engineering issue to file. |
| R7 | Every new on-chain field or event must pass the CONTRIBUTING checklist and update this inventory. | K7 | Done in this PR (CONTRIBUTING checklist). |
| R8 | Operators (admin, signers) should be organisational multisigs, not personal keys. | Admin rows | Accepted risk (proposed): already the recommended deployment. |

## 7. Maintaining this document

Any PR that adds or changes a `#[contracttype]` stored under a `DataKey`, or a
`#[contractevent]`, must update section 3 and, if risk changes, section 5.
See the checklist in CONTRIBUTING.md.
