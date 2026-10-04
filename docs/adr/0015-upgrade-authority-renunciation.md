# ADR-0015: Upgrade-Authority Renunciation Path (Immutability Mode)

- **Status:** Proposed
- **Date:** 2026-09-26
- **Deciders:** Maintainers and relevant reviewers

## Context

The `attester-registry::upgrade()` function allows the admin to replace the contract's code at any time. For pre-alpha and early testnet, this is the right design: rapid iteration, hotfixes, and experimental features require upgradeability. However, for mature mainnet deployments, permanent upgradeability is itself a trust assumption: anyone controlling the admin quorum can rewrite the meaning of "verified" at any time, unobservably.

Health authorities, auditors, and regulators may require that once the verification logic is audited and deployed to production, it becomes immutable. Code changes after that point require an explicit migration to a new contract, with observable state transfer and verifiers' deliberate consent to trust the new contract.

This ADR specifies an irreversible "renunciation" mechanism that the admin can use to lock in the current code and prevent all future upgrades.

## Decision

Implement upgrade-authority renunciation in `attester-registry` and `attestation-registry`:

### Core Operations

```rust
#[contracttype]
enum DataKey {
    // ... existing keys ...
    UpgradesRenounced, // bool; once true, remains true forever
}

/// Propose to renounce upgrade authority. Requires admin auth.
/// Sets ProposedRenunciation to true and emits UpgradesRenounceProposed.
pub fn propose_renounce_upgrades(env: Env) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    env.storage()
        .instance()
        .set(&DataKey::ProposedRenunciation, &true);
    env.storage()
        .instance()
        .set(&DataKey::ProposalLedger, &env.ledger().sequence());
    env.events().publish(
        (Symbol::new(&env, "UpgradesRenounceProposed"),),
        (env.ledger().timestamp(),),
    );
    Ok(())
}

/// Confirm renunciation after a minimum delay (e.g., 7 days ≈ ~100,000 ledgers).
/// Requires admin auth again. Once confirmed, UpgradesRenounced = true permanently.
/// This prevents a single compromised transaction from locking in renunciation.
pub fn confirm_renounce_upgrades(env: Env, min_ledgers: u32) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    
    let proposed: bool = env.storage().instance().get(&DataKey::ProposedRenunciation)
        .unwrap_or(false);
    if !proposed {
        return Err(Error::NoProposedRenunciation);
    }

    let proposal_ledger: u32 = env.storage().instance().get(&DataKey::ProposalLedger)
        .unwrap_or(0);
    let current_ledger = env.ledger().sequence();
    
    if current_ledger < proposal_ledger + min_ledgers {
        return Err(Error::RenunciationTooSoon);
    }

    env.storage()
        .instance()
        .set(&DataKey::UpgradesRenounced, &true);
    env.storage().instance().remove(&DataKey::ProposedRenunciation);
    env.storage().instance().remove(&DataKey::ProposalLedger);

    env.events().publish(
        (Symbol::new(&env, "UpgradesRenounced"),),
        (env.ledger().timestamp(),),
    );
    Ok(())
}

/// Check if upgrades have been renounced.
pub fn is_upgradeable(env: Env) -> bool {
    let renounced: bool = env.storage().instance().get(&DataKey::UpgradesRenounced)
        .unwrap_or(false);
    !renounced
}

/// Upgrade the contract code. Fails with Error::UpgradesDisabled if renounced.
pub fn upgrade(env: Env, new_wasm_ref: BytesN<32>) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    
    if !Self::is_upgradeable(&env) {
        return Err(Error::UpgradesDisabled);
    }

    // ... existing upgrade logic ...
}
```

### Why `migrate()` is Also Disabled

The `migrate()` function (if present) is used to transform storage keys when upgrading. If upgrades are renounced, `migrate()` must also be disabled:
- A compromised admin could call `migrate()` to alter historical state without changing code
- `migrate()` at its core is a state transform, similar in power to `upgrade()`
- Disabling both ensures that renunciation truly locks in both code AND state interpretation

### Two-Step Safety Latch

The two-step process (`propose` → wait N ledgers → `confirm`) provides:
- **Time window:** The community, multi-sig guardians, and monitoring systems see the proposal and can react if it's unauthorized
- **Unwind window:** If the proposal is a mistake, the admin can let the proposal expire without confirming
- **Irreversibility check:** By requiring auth twice, we ensure the action is deliberate, not a one-off mistake

If renunciation happens without authorization, the admin still has time between proposal and confirmation to investigate and respond.

## Consequences

### Positive

- Audited releases can be locked in; code can't be changed without observable migration
- Meets regulatory and auditor requirements for immutable verification logic
- Two-step process prevents accidental renunciation
- Clear operational policy enables trust from health authorities

### Trade-offs and risks

- Once renounced, the contract is forever frozen. Critical bugs found later can't be hotfixed—only a full migration to a new contract is possible
- Renunciation is irreversible; a mistaken renunciation locks the deployment permanently
- The two-step delay (N ledgers) is a UX cost; operational emergencies can't immediately renounce
- Verifiers must track both old and new contract addresses if a migration is triggered by a critical bug

## Operational Policy

### When to Renounce (ADR section: Policy)

- **Never on testnet:** Testnet is for iteration; upgradibility is essential
- **Only after audited release:** Proposal is made only after an independent audit of code and threat model
- **After stability period:** Wait at least X months (e.g., 3-6 months) of bug-free operation in production before renouncing
- **With stakeholder buy-in:** Health authorities, program leads, and key verifiers must agree to the lock-in

### How to "Upgrade" After Renunciation

- **Deploy a new contract:** `attester-registry-v2` with the required changes
- **State migration:** Export state from v1, transform as needed, import into v2 (see ADR-0016)
- **Dual-contract period:** For a transition window (e.g., 30 days), verifiers accept attestations from both v1 and v2
- **Gradual deprecation:** v1 is marked deprecated in events; admin gradually removes attesters from v1
- **Full cutover:** After stability, v2 becomes the canonical registry; v1 is retired

## Follow-up

1. Implement two-step renunciation in `attester-registry` and `attestation-registry`
2. Ensure `migrate()` (if exists) is also gated by `UpgradesRenounced` flag
3. Add comprehensive tests: proposal, delay verification, confirmation, upgrade-after-renounce rejection
4. Document the operational policy in CONTRIBUTING.md and an ADR section
5. Implement `is_upgradeable()` query for verifiers to check if a contract is locked
6. Add runbook: "How to Renounce Upgrades" and "How to Migrate After Renunciation"

## References

- Issue #368: Add upgrade-authority renunciation path
- Related: ADR-0016 (registry re-genesis and state migration)
- Soroban upgrade safety: https://stellar.org/docs/build/smart-contracts/managing-state
