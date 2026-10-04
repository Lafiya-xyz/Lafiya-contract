# ADR-0014: Patient-Controlled Card Pointer Registry

- **Status:** Proposed
- **Date:** 2026-09-26
- **Deciders:** Maintainers and relevant reviewers

## Context

QR codes printed on patient cards encode critical information. Today, if they encode a record hash (a commitment hash or full record hash), every update to the patient's medical record (new medication, corrected allergy, status change) invalidates all printed QR codes. A patient carrying an old printout still sees an old, possibly stale record. If QR codes instead encode a mutable URL (e.g., `https://lafiya.health/patient/uuid`), trust is delegated to lafiya-web's server rather than remaining on-chain and immutable.

A stable, patient-controlled on-chain pointer solves this: a unique `card_id` (128+ bits of entropy) resolves to the current record hash. When the record updates, the patient's card app updates the pointer on-chain, and all old QR printouts immediately resolve to the current record without needing new printouts.

## Decision

Deploy a `card-registry` contract on-chain providing:

### Core Operations

```rust
pub fn register_card(
    env: Env,
    owner: Address,      // Authenticate as this address
    card_id: BytesN<32>, // Random 32-byte value (128+ bits entropy)
    record_hash: BytesN<32>,
) -> Result<(), Error> {
    owner.require_auth();
    // Store mapping: card_id -> record_hash
    // Emit CardRegistered(owner, card_id, record_hash)
}

pub fn update_card(
    env: Env,
    card_id: BytesN<32>,
    new_record_hash: BytesN<32>,
) -> Result<(), Error> {
    // Caller must be the original owner
    // Update mapping: card_id -> new_record_hash
    // Keep bounded history (e.g., last 5 updates)
    // Emit CardUpdated(card_id, old_hash, new_hash)
}

pub fn resolve(env: Env, card_id: BytesN<32>) -> Option<BytesN<32>> {
    // Open read, no auth required
    // Return current record hash for this card_id
}

pub fn get_card_history(
    env: Env,
    card_id: BytesN<32>,
) -> Vec<CardUpdate> {
    // Return bounded history of (timestamp, old_hash, new_hash) tuples
}

pub fn transfer_card_owner(
    env: Env,
    card_id: BytesN<32>,
    new_owner: Address,
) -> Result<(), Error> {
    // Current owner auth required
    // Transfer ownership; new_owner can now call update_card
    // Emit CardOwnerTransferred(card_id, old_owner, new_owner)
}
```

### Privacy Design

- `owner` should be a **per-card derived key**, not the patient's primary account
- Derivation: `owner = sha256(patient_main_account || card_id)` (or similar deterministic secret)
- This prevents an observer from linking multiple cards to the same patient even if they know the patient's main account
- The patient's app knows the derivation secret and can compute `owner` for any `card_id` it knows

### Storage & History

- **Primary:** `CardRegistry(card_id) -> (owner, current_record_hash, last_updated_ledger)`
- **History:** `CardHistory(card_id, index) -> (old_hash, new_hash, timestamp)` — keep last N updates (e.g., 5)
- History size is bounded to control rent costs
- Older history is prunable by the owner if rent becomes an issue

### Verifier Flow (End-to-End)

1. **Scan:** Verifier scans QR code encoding `card_id` (not record hash)
2. **Resolve:** Verifier calls `resolve(card_id)` to get current `record_hash`
3. **Fetch:** Verifier fetches the full record from IPFS/off-chain using `record_hash`
4. **Verify:** Verifier checks the record's signature against the attestation registry
5. **Trust:** Trust chain: card_id (immutable, on-chain pointer) → record_hash (patient-updated via card app) → record content (IPFS) → attestation signature (on-chain registry)

### Interaction with Record Supersession

When a record is superseded (a newer version replaces an older one):
- Patient's card app detects the supersession and calls `update_card` with the new `record_hash`
- Old `record_hash` moves into history
- Verifiers querying the old `record_hash` still see it in history but should recognize it as superseded by checking the `CardUpdated` events

## Alternatives Considered

### A. Signed QR Payload Only

Encode the entire record (or a commitment + timestamp) in the QR code, signed by the patient's key. The verifier verifies the signature on-chain.

**Rejected because:**
- QR codes have limited capacity (~2KB); a full record + signature + timestamp is tight
- A signed record is a snapshot; any update requires a new printout
- No support for record correctionsor retroactive updates to old printouts

### B. Off-Chain URL Pointers

QR codes encode a URL like `https://lafiya.health/card/card_id`. lafiya-web dereferences it to the current record.

**Rejected because:**
- Trust is delegated to a centralized server (lafiya-web)
- If the server is compromised or offline, cards are useless
- No on-chain audit trail of updates
- Doesn't align with Lafiya's decentralized verification principle

### C. Smart Contract on Attestation Registry

Store card pointers in the `attestation-registry` contract alongside attestations.

**Rejected because:**
- Conflates two concerns: attestations and card pointers
- Increases attestation-registry complexity and WASM size
- Card management shouldn't require admin permissions on attestations

## Consequences

### Positive

- QR codes become immutable and reusable for the patient's lifetime
- Record updates don't require new printouts
- On-chain audit trail of all pointer updates via `CardUpdated` events
- Privacy-preserving: cards can't be linked by observers if per-card owner keys are used
- Decentralized: verifier can resolve pointers without trusting a server

### Trade-offs and risks

- New contract increases operational complexity
- Rent costs accrue for every active card (must be measured)
- Patient must keep their card app running to maintain the pointer (if card app is lost, card becomes stale)
- History is bounded; patients who update frequently may lose old history
- If a patient's derived key is compromised, the attacker can update the pointer to point to fraudulent records

### Rent Analysis

A single card entry:
- `CardRegistry(card_id) -> (owner: 32B, hash: 32B, timestamp: 8B)` ≈ 75 bytes per card
- With soroban rent at ~0.5 stroops/byte, a card costs ~37 stroops per ledger to keep alive
- Over 1 year (52,560 ledgers), ≈ 1.9M stroops (negligible in USD)
- Over 10 years, ≈ 19M stroops (also negligible)
- Bounded history (5 updates × 75B ≈ 375B total) is affordable

## Follow-up

1. Implement `card-registry` contract with register, update, resolve, transfer, and history functions
2. Implement per-card derived-key privacy model; document the derivation in README
3. Add comprehensive tests: registration, updates, history, owner transfer, privacy
4. Write verifier integration guide with code example (scan → resolve → verify flow)
5. Implement `lafiya-web` integration: card app calls `update_card` when records change
6. Document recovery path if patient loses card device: caregiver/guardian intervention
7. Measure rent costs on testnet; adjust history size if needed
8. Publish rent cost analysis in docs/

## References

- Issue #367: Add a patient-controlled card pointer registry
- Related: ADR-0001 (hash-only on-chain footprint) — similar privacy-by-hash pattern
- Related: Attestation registry for record verification
- Soroban rent: https://stellar.org/docs/learn/storing-data
