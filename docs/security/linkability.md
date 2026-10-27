# Patient linkability through public record-hash activity

- **Status:** Analysis
- **Decision:** [ADR-0012](../adr/0012-per-attestation-blinded-commitments.md)
- **Proof of concept:** [`scripts/poc/linkability_poc.py`](../../scripts/poc/linkability_poc.py)

Lafiya keeps only hashes on-chain ([ADR-0001](../adr/0001-hash-only-on-chain-footprint.md))
and salts *record commitments* ([ADR-0008](../adr/0008-record-commitment-canonicalization.md)).
That hides record **contents**, but the **pattern** of on-chain activity around a
commitment still leaks information. This document quantifies each leak and evaluates
mitigations.

## Threat model

- **Observer:** anyone who can read the public ledger (block explorer, indexer, RPC).
- **Side knowledge:** the observer may have scanned a patient's card once (a clinic
  worker, a checkpoint, anyone shown the QR code), and knows which region or facility
  each *attester* address belongs to (attester addresses are public and often discoverable).
- **Goal:** link on-chain events to a person and learn their movements, visit timing,
  record history, or a *CHW*'s workload.

## Channels

Severity: **High** = one card scan is enough to track a person over time; **Medium** =
needs extra side knowledge or reveals aggregate data; **Low** = small marginal leak.

| # | Channel | What the observer learns | Side knowledge needed | Severity |
|---|---|---|---|---|
| 1 | **Watch a hash:** the card's `record_hash` repeats on every re-attestation and revocation (and on access-log entries, if built). | Every future and past visit to a verification point: when, and through which attester. | One card scan. | **High** |
| 2 | **Timestamps + attester region** on each attestation. | The patient's movements between regions and facilities over time. | Channel 1, plus a mapping of attester to region (often public). | **High** |
| 3 | **Supersession chains** (if built): a new record links to the one it replaces. | Links successive records of one patient, so rotating the record does not break tracking. | Any one record hash in the chain. | **Medium** (future) |
| 4 | **Attester activity stream:** all attestations by one CHW address. | How many patients a CHW verified and when; in conflict-affected areas this exposes CHW operations and patterns. | The CHW's address (public). | **Medium** |
| 5 | **Fee-payer / source account** of the submitting transaction. | Links attestations submitted from the same device or account even across attesters. | None. | **Low** |

The PoC reproduces channels 1 and 2 against a local sandbox event log: with 200 patients
and 6 visits each, a single scanned hash links all 6 of the victim's attestations and
reveals the regions visited.

## Mitigations

| Mitigation | Breaks | Cost / trade-off |
|---|---|---|
| **Per-attestation blinded commitments:** each attestation commits to `H(domain ‖ record ‖ fresh_nonce)`; the nonce is carried on the card and used during verification. | 1, 2 (no repeating hash to watch), 3 (if supersession links blinded values). | New card must be issued per attestation; revocation must target the specific blinded hash; the registry can no longer list a record's history by one key (history lives off-chain with the patient). |
| **Card-scoped rotating identifiers** in the QR code. | Linking across cards if the QR identifier is not the on-chain key. | Only helps if the on-chain key also rotates; alone it just moves the linkable value. Needs a resolver service, which becomes a tracking point. |
| **Batch anchoring** (Merkle roots of many attestations). | 2 partly (timing hidden inside the batch window); 4 partly. | Verification needs a Merkle proof on the card; revocation needs a separate structure; latency equals the batch window; significant contract changes. |
| **Relayed submission** through a shared relayer account. | 5 (source account). | Operates a relayer (availability, fees, trust); does **not** hide the attester address inside the attestation. |
| **Coarse timestamps** (rounded to the day). | 2 partly (fine-grained timing). | Weakens compromise windows ("attested after T") to day precision; ledger close time is still public at the transaction level, so it only helps together with batching. |

## Recommendation

Adopt **per-attestation blinded commitments** (ADR-0012) as the primary mitigation: it
removes the highest-severity channel (watch a hash) with a contained change to the commitment
format and no new infrastructure. The PoC shows the attack linking 1 event instead of 6.
Pair it with relayed submission later for channel 5, and track batch anchoring as a
research item for channels 2 and 4.

## Privacy review: dispute flags and status lists

Two planned features add new on-chain signals. Both are reviewed here against the
channels above before implementation.

### Patient-initiated dispute flags (issue #546)

A patient-signed `Disputed` overlay (keyed by record hash and signer) lets a patient
flag incorrect verified data without deleting it. The dispute itself is a new
linkable signal:

- **Channel 1 (watch a hash) is re-opened for disputed records.** A dispute is
  submitted against a specific record hash, so an observer who scanned the card can
  see that the patient disputed it, and when. Under per-attestation blinded
  commitments the disputed hash is the blinded value, which limits the leak to that
  one attestation, but the dispute transaction still ties the patient's action to a
  verification point in time.
- **Dispute timing correlates with visits.** A dispute is typically filed shortly
  after a patient learns of the bad record (often right after a visit), so dispute
  timestamps leak coarse movement information (channel 2).
- **Dispute spam is itself a signal.** Unbounded disputes would let an observer
  distinguish active patients and would also be a griefing vector; disputes must be
  bounded per signer/record and clearing must require attester or admin action
  (anti-abuse requirement in #546).

**Guidance:** disputes should be submitted through the same relayed-submission path
as attestations (channel 5) where available, and the dispute signature domain must be
separated from attestation and revocation domains so a dispute signature cannot be
replayed as another action. Disputes are documented as an intentional, bounded
linkability trade-off: the patient chooses to surface the signal.

### Status-list contract (issue #547)

A bitstring status list (aligned with W3C Bitstring Status List) lets verifiers sync
revocation state in bulk instead of per-hash lookups. The privacy risk is **index
correlation**:

- **Index allocation must not reveal record identity.** If the status-list index were
  derived directly from the record hash (or assigned in attestation order), an
  observer could map list positions back to records and reconstruct which entries
  were revoked, re-creating channel 1 at list granularity.
- **Correlation across syncs.** A verifier that caches successive list versions can
  diff them; if indices are stable and ordered by creation time, the diff reveals
  *when* records were revoked and how many, which correlates with attester activity
  (channel 4).
- **Segment/commitment metadata.** Versioned segment commitments must not embed
  record hashes or attester addresses in plaintext; only the signed root and segment
  version are published.

**Guidance:** allocate indices via a keyed, unpredictable mapping (e.g. a
domain-separated PRF over the blinded record hash with a registry-held key) so
positions carry no ordering or identity information; publish only versioned
commitments and signed roots; and document the residual leak that the *count* of
revocations per segment is public. The threat analysis and measured size/sync time
for 1M entries are tracked in #547.

## Follow-up engineering issues

1. Extend LRC-1 with a per-attestation nonce (`LRC-2`) and update `lafiya-commitment` and the CLI.
2. Carry the nonce in the card payload; update the verifier and the [verification model](../specs/verification-model.md) inputs.
3. Revocation by blinded hash, and an off-chain (patient-held) attestation history.
4. Relayed submission for attesters (channel 5).
5. Research spike: batch anchoring and CHW activity privacy (channels 2 and 4).
6. Dispute flags (#546): domain-separated patient signature, bounded disputes, attester/admin clearing.
7. Status-list index allocation (#547): PRF-based indices, versioned commitments, index-correlation analysis.
