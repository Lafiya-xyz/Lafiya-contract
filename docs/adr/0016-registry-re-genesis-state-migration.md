# ADR-0016: Registry Re-Genesis — Migrating State to a Fresh Deployment

- **Status:** Proposed
- **Date:** 2026-09-26
- **Deciders:** Maintainers and relevant reviewers

## Context

There are scenarios where a deployment must be abandoned and restarted from a fresh contract:
- **Admin quorum lost:** All private keys are compromised or inaccessible; no recovery path
- **Upgrades renounced + critical bug:** After renunciation (ADR-0015), a critical bug is discovered; can't be hotfixed
- **Hijacked initialization:** An early deployment was misconfigured by an attacker during initialization
- **Protocol change:** A future Soroban protocol version is incompatible with the old contract

Today there is no plan for re-genesis. A new deployment starts with an empty allowlist and zero attestations. Every card in the field would show as unverified, destroying trust and requiring manual re-attestation.

This ADR specifies:
1. Which state carries over (allowlist, attestation history, revocations) and which doesn't
2. How verifiers can trust that migrated state is faithful (not fabricated by the new admin)
3. A transition period for dual-contract operation
4. Tooling and runbooks for the full procedure

## Decision

### State Scope

**Carries over to new contract:**
- **Allowlist:** All attester entries (address, license_hash, region) with addition timestamps
- **Attestation history:** All attestation events (who attested what, when, signature)
- **Revocations:** All revoked attestations with revocation timestamps

**Does NOT carry over:**
- **Admin quorum configuration:** New contract has a fresh admin (chosen by governance)
- **Pause state:** New contract starts unpaused; pause decisions are fresh
- **TTL/rent metadata:** Internal Soroban state; not semantically meaningful to migrate
- **Schema version:** New contract starts at v1

Rationale: Attesters and attestations are domain facts; they should survive. Admin configuration is a governance decision tied to the old deployment and is remade with new governance.

### Trust Model: Migrated State Attestation

When verifiers query the new contract, they must be able to verify that migrated attestations are faithful copies, not fabrications by the new admin.

**Option A: Migrated_From Field**

Each attestation stores:
```rust
pub struct AttestationRecord {
    // ... existing fields ...
    pub migrated_from: Option<(Address, Ledger)>,  // old_contract_address, old_ledger_sequence
    pub migration_signature: Option<BytesN<64>>,   // Signature of old admin attesting the migration
}
```

- Old admin signs the attestation set before migration
- New admin includes the signature in migrated records
- Verifier can call old contract (read-only) to confirm the signature and attestation
- Downside: requires old contract to remain readable; doesn't work if old contract is deleted

**Option B: Merkle Snapshot Published On-Chain**

Before migration, compute a Merkle root of all attestation state and publish it in the old contract (or via a separate "migration anchor" contract):
```rust
pub struct MigrationSnapshot {
    pub root: BytesN<32>,  // Merkle root of attestations
    pub old_contract: Address,
    pub timestamp: u64,
}
```

- New contract includes `migration_snapshot` field
- Verifier checks any migrated attestation against the published Merkle root
- Trustworthy: root is published before migration, so it can't be altered retroactively
- Downside: complex Merkle tree construction; moderate off-chain tooling overhead

**Option C: Dual-Contract Trust Period**

For a fixed transition window (e.g., 30 days), verifiers check BOTH the old and new contracts:
- Query old contract first (canonical, immutable state)
- If not found, query new contract (fresh, potentially fabricated)
- After transition, deprecate the old contract; all trust shifts to new

Rationale: No cryptographic proof needed. The old contract is read-only during transition; if attestations don't exist there, they weren't in the old deployment and shouldn't be trusted.

**Recommendation: Option C + A hybrid**

Use Option C for normal operation (dual-contract trust period) and Option A for long-term verification (migrated_from signature). This gives:
- Operational simplicity during transition (just check both contracts)
- Long-term trust after old contract is deprecated (signature proof)
- Graceful fallback if new admin is compromised (verifiers can revert to checking old contract only)

### Migration Procedure

#### Phase 1: Preparation (governance, ~7 days)
1. Governance votes to approve re-genesis and selects new admin
2. Snapshot tooling exports full state from old contract
3. Snapshot is published and hashed; hash is announced publicly
4. Community validates snapshot (compare against their own records)

#### Phase 2: New Contract Deployment
1. Deploy new `attester-registry-v2` and `attestation-registry-v2` (fresh, empty)
2. New contracts have empty admin (set by governance vote) and zero attestations
3. New contracts are NOT yet the canonical registries; old contracts remain in use

#### Phase 3: State Import
1. Call `import_attestations(attestation_vec)` on new contract (admin-only, one-time only)
   - Import all attestations, revocations, and allowlist from snapshot
   - Verify snapshot hash matches announced hash
   - Emit `StateImported(snapshot_hash, count)`
2. After import, call `seal_import()` (admin-only, one-time) to lock import window
   - After sealing, `import_attestations()` reverts
   - Prevents later fabrication of additional "migrated" attestations

#### Phase 4: Dual-Contract Period (~30 days)
1. Verifiers are updated to check BOTH old and new contracts
   - Query new contract first (lower latency)
   - If not found, query old contract
   - Accept attestations from either
2. Monitoring: track usage and errors
3. CHWs, health authorities, and key integrations run parallel checks

#### Phase 5: Cutover
1. After stability period, declare new contract canonical
2. Old contract is marked deprecated in events
3. Verifiers can now check only new contract (old contract checked only for legacy fallback)
4. Old contract remains read-only for up to 1 year (for historical verification)

### Storage Implementation

#### New Contract: One-Time Import Window

```rust
#[contracttype]
enum DataKey {
    // ... existing keys ...
    ImportSealed,  // bool; once true, import is permanently closed
    ImportedSnapshot, // BytesN<32>; hash of snapshot that was imported
}

pub fn import_attestations(
    env: Env,
    attestations: Vec<AttestationRecord>,
    snapshot_hash: BytesN<32>,
) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    
    // Check import not already sealed
    let sealed: bool = env.storage().instance().get(&DataKey::ImportSealed)
        .unwrap_or(false);
    if sealed {
        return Err(Error::ImportSealed);
    }

    // Validate snapshot hash matches one announced via governance
    let expected_hash = Self::announced_snapshot_hash(&env)?;
    if snapshot_hash != expected_hash {
        return Err(Error::SnapshotMismatch);
    }

    // Import attestations
    for attestation in attestations.iter() {
        // Store with migrated_from metadata
        let mut att = attestation.clone();
        att.migrated_from = Some((old_contract_addr, old_ledger_sequence));
        env.storage().persistent().set(
            &DataKey::Attestation(att.id),
            &att,
        );
    }

    // Import allowlist
    // (details similar)

    env.events().publish(
        (Symbol::new(&env, "StateImported"),),
        (snapshot_hash, attestations.len()),
    );
    Ok(())
}

pub fn seal_import(env: Env) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    
    let sealed: bool = env.storage().instance().get(&DataKey::ImportSealed)
        .unwrap_or(false);
    if sealed {
        return Err(Error::AlreadySealed);
    }

    env.storage().instance().set(&DataKey::ImportSealed, &true);
    env.events().publish(
        (Symbol::new(&env, "ImportSealed"),),
        (env.ledger().timestamp(),),
    );
    Ok(())
}
```

## Snapshot & Import Tooling

### Snapshot Export (CLI Command)

```bash
lafiya-cli export-snapshot \
  --rpc https://rpc.stellar.org \
  --old-contract CCCCCC... \
  --output snapshot.json
```

Produces canonical JSON:
```json
{
  "exported_at_ledger": 12345,
  "attestations": [
    {
      "id": "0x...",
      "attester": "GXXXX",
      "record_hash": "0x...",
      "timestamp": 1234567890,
      "signature": "0x..."
    }
  ],
  "allowlist": [
    {
      "attester": "GXXXX",
      "license_hash": "0x...",
      "region": "US"
    }
  ],
  "revocations": [
    {
      "attestation_id": "0x...",
      "revoked_at": 1234567890
    }
  ]
}
```

Compute snapshot hash: `sha256(canonical_json_bytes)`

### Import CLI

```bash
lafiya-cli import-snapshot \
  --rpc https://rpc.stellar.org \
  --new-contract CCCCCC... \
  --snapshot snapshot.json \
  --admin-key /path/to/key \
  --sign
```

This calls the new contract's `import_attestations()` function with the snapshot.

## Runbook: Full Re-Genesis Procedure

### Pre-Genesis (Governance Phase)
1. Governance proposal: "Migrate to new registry due to [reason]"
2. Vote passes; new admin is elected
3. Timeline announced: snapshot export → new deployment → dual-contract period → cutover

### Export Snapshot
1. Run `lafiya-cli export-snapshot` pointing to old contract
2. Compute snapshot hash publicly
3. Publish hash (announcement, GitHub, emails)
4. Community members validate locally and confirm hash

### Deploy New Contract
1. Deploy fresh `attester-registry-v2` and `attestation-registry-v2`
2. Set new admin (elected by governance)
3. Announce new contract addresses

### Import State
1. Admin calls `import_attestations(snapshot.json, hash)` on new contract
2. Monitor import for errors; verify counts match snapshot
3. Admin calls `seal_import()` to lock the import window
4. Emit "ImportSealed" event; announce seal

### Dual-Contract Period
1. Update verifier endpoints: check new contract, fall back to old
2. CHWs and key integrations test with new contract
3. Monitor for divergence (shouldn't happen if import was faithful)
4. Duration: typically 30 days; can extend if issues found

### Cutover
1. After stability, declare new contract canonical
2. Mark old contract deprecated in events
3. Update verifier configs to check new contract only
4. Old contract remains read-only for historical fallback (1+ years)

## Consequences

### Positive

- Enables recovery from catastrophic failures without destroying trust
- Preserves attestation history and allowlist across re-genesis
- Dual-contract period allows gradual confidence-building
- Tooling makes the process operationally feasible and auditable

### Trade-offs and risks

- Complex procedure; requires coordination across multiple stakeholders
- Snapshot export tooling must be carefully implemented; bugs could lead to incomplete or fraudulent snapshots
- Dual-contract period is operationally expensive; verifiers must check two contracts
- If the new admin is compromised before import is sealed, attestations could be fabricated
- Long-term verifiers need to understand when the cutover happened and trust the new contract accordingly

## Follow-up

1. Implement `import_attestations()` and `seal_import()` in both registries
2. Build CLI snapshot export and import tooling
3. Test re-genesis procedure on testnet end-to-end
4. Document full runbook in CONTRIBUTING.md
5. Plan historical audit: keep old contract read-only for verification
6. Implement monitoring dashboard: track dual-contract period, attestation divergence
7. Prepare communication plan for stakeholders (CHWs, health authorities, verifiers)

## References

- Issue #370: Registry re-genesis and state migration
- Related: ADR-0015 (upgrade renunciation), which can trigger re-genesis
- Related: Timelock controller (ADR discussion) for safe admin transitions
- State archival and migration: https://stellar.org/docs/learn/storing-data
