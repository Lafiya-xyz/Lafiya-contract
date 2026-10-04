# Access-Log Contract Design

**Issue:** #366 — Break-glass access log for patient-auditable emergency record access

## Overview

The Lafiya system allows responders to read an unconscious patient's emergency data without consent at the moment of access (break-glass pattern). Under NDPA 2023 and good health privacy practice, the patient must be able to audit who accessed their data and when. Currently, no access trail exists.

This document specifies a new `access-log` contract that records privacy-preserving access events.

## Design Goals

1. **Patient privacy:** No observer can link access events to a patient identity or to specific attestation record hashes
2. **Responder accountability:** Allowlisted responders must cryptographically sign each access event
3. **Auditability:** Patients can retrieve their own access log by recomputing a deterministic secret-derived commitment
4. **Incident transparency:** Break-glass access must be logged; no events are anonymous

## Architecture

### Core Data Structures

```rust
pub struct AccessEvent {
    pub id: u64,                          // Incremental event counter
    pub card_commitment: BytesN<32>,      // SHA-256 hash, patient-specific & time-gated
    pub context: AccessContext,           // Enum: EmergencyScan, ClinicIntake, Verification, etc.
    pub responder: Address,               // Allowlisted facility or responder
    pub timestamp: u64,                   // Ledger timestamp of access
    pub record_hash: Option<BytesN<32>>,  // Optional: which record was accessed (if safe to log)
}

pub enum AccessContext {
    EmergencyScan = 1,       // Roadside emergency responder scanning card
    ClinicIntake = 2,        // Clinic staff accessing during patient intake
    Verification = 3,        // Emergency verification (e.g., blood type check)
    AdminReview = 4,         // Administrative audit access
}

pub struct SignedAccessEvent {
    pub event: AccessEvent,
    pub responder_signature: BytesN<64>,  // Ed25519 signature by responder's key
    pub nonce: u64,                       // Prevent replay attacks
}
```

### Key Security Properties

#### Card Commitment (Hash-Only Linking)

The `card_commitment` is computed as:
```
card_commitment = SHA-256("lafiya:access:v1" || card_secret || epoch)
```

Where:
- `card_secret`: A value known only to the patient's hardware card and `lafiya-web` (NOT stored on-chain)
- `epoch`: A time window (e.g., Unix timestamp // 86400, one per day) — ensures old commitments become un-linkable
- Hashing one-way prevents any observer from deriving `card_secret` or `epoch` from the logged commitment
- Same `card_secret` + `epoch` pair produces the same `card_commitment` — allows patient queries

#### Responder Authorization

- Only allowlisted responders (queried from `attester-registry` or a separate responder registry) can log events
- Each responder cryptographically signs the event; tampering with the log is detectable
- Anonymous scans by `lafiya-web` are allowed (web signs for itself) but MUST be marked as such; this weakens accountability and must be documented as a risk

#### Patient Retrieval

The patient retrieves their own access log off-chain:
1. For each `epoch` of interest (e.g., today, yesterday, last week):
   - Compute `card_commitment = SHA-256("lafiya:access:v1" || patient_card_secret || epoch)`
2. Query the contract: `get_access_events_by_commitment(card_commitment) -> Vec<AccessEvent>`
3. Verify each event's signature using the responder's known public key

### Contract Interface

```rust
pub fn log_access(
    env: Env,
    card_commitment: BytesN<32>,
    context: AccessContext,
    responder: Address,
    signature: BytesN<64>,
    nonce: u64,
) -> u64 {
    // responder.require_auth() — responder must sign the transaction
    // Verify responder is allowlisted (or is lafiya-web)
    // Verify signature over (card_commitment, context, responder, nonce)
    // Check nonce is not a replay (store in set or map)
    // Increment event counter and store
    // Emit AccessLogged event
    // Return event_id
}

pub fn get_access_events_by_commitment(
    env: Env,
    card_commitment: BytesN<32>,
) -> Vec<AccessEvent> {
    // No authorization required (events are already privacy-filtered by commitment)
    // Retrieve all events matching this commitment
    // Typically 0–100 events per patient per day depending on emergency activity
}

pub fn get_access_event(env: Env, event_id: u64) -> Option<AccessEvent> {
    // Returns a single event by ID
    // No authorization; commitment already hides the patient
}
```

### Storage Strategy

**Decision: Use Soroban contract events for indexing, optional persistent storage for recent queries.**

- **Primary:** Emit a `AccessLogged` event for each access. Off-chain indexers (e.g., Stellar Indexer) consume and persist events.
  - Advantage: Cheap on-chain; events are append-only and never modified
  - Disadvantage: Requires operational indexing; patients must query indexed data, not contract state directly
  
- **Secondary (if audit retention required):** Store recent events (e.g., last 90 days) in contract persistent storage as a fallback.
  - Rent costs: At ~0.5 stroops/byte, 90 days × 50 events/patient × 200 bytes/event ≈ 4,500 stroops per patient per quarter
  - Trade-off: Minimal cost for medium-term auditability; older events rely on indexer

**Recommendation:** Implement events-only initially. Add persistent storage if audits reveal retention gaps.

### Risk Mitigations

#### Risk: "An attacker logs fake events under a patient's commitment"
**Mitigation:** 
- Only allowlisted responders (with registered keys) can call `log_access`
- Each event is signed; fake entries are detectable
- Patient can cross-check against facility records or known responder identities

#### Risk: "Epoch window is too large; commitment re-links across time"
**Mitigation:** 
- Use small epochs (24 hours recommended; responders see new commitments daily)
- Document in README that commitments older than 30 days are considered "de-linked" and stale

#### Risk: "Card secret is compromised; attacker links all a patient's events"
**Mitigation:** 
- This is equivalent to compromising the patient's hardware card — a fundamental trust assumption
- If card is compromised, patient should rotate secrets via `lafiya-web`
- Document: "Breach of card secrets compromises all access logs; patients MUST rotate card secrets"

#### Risk: "Anonymous (lafiya-web) access reduces accountability"
**Mitigation:** 
- Mark all web-signed events with a `responder = SYSTEM_LAFIYA_WEB` constant
- Document in README: "Anonymous web access is logged but cannot be attributed to a specific responder; use sparingly"
- Consider separate event type or context for anonymous access
- Audit web access patterns separately (e.g., "web accessed this commitment at 3am" may indicate testing/abuse)

## Implementation Phases

### Phase 1: Core Contract
- Implement `log_access`, `get_access_events_by_commitment`, `get_access_event`
- Emit events on each access
- Simple in-memory nonce tracking for replay prevention
- No persistent storage; rely on event indexing

### Phase 2: Integration
- Wire `attestation-registry::attest` to call `access-log::log_access` when serving emergency data
- Update responder allowlisting (if separate from attester registry) to include access-log auditing
- Deploy and test with dev/test responders

### Phase 3: Monitoring & Retention
- Deploy Stellar Indexer to consume and persist `AccessLogged` events
- Evaluate rent costs; decide if persistent storage is needed
- Document patient audit flow in runbook

## Documentation

Update the following:
- **README:** Add "Emergency Record Access Audit" section explaining how patients retrieve their logs
- **SECURITY.md:** Note that card secrets are high-value; compromise requires immediate rotation
- **Runbook:** Add "Patient Requests Access Audit" procedure
- **Architecture:** ADR-0013 (if a longer-term decision is needed on storage/indexing)

## References

- Issue #366: Break-glass access log contract
- Related: ADR-0001 (hash-only on-chain footprint) — similar privacy-by-hash pattern
- Related: NDPA 2023, Health Information Protection standards
- Stellar Indexer: https://developers.stellar.org/docs/learn/storing-data
