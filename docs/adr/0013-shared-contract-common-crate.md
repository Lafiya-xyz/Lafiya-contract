# ADR-0013: Extract Shared Admin, Pause, and TTL Logic into lafiya-contract-common

- **Status:** Proposed
- **Date:** 2026-09-26
- **Deciders:** Maintainers and relevant reviewers

## Context

Both `attester-registry` and `attestation-registry` contain identical or near-identical implementations of:
1. Admin management: `propose_admin()`, `accept_admin()`, `admin()`
2. Pause control: `pause()`, `unpause()`, `is_paused()`, `require_not_paused()`
3. TTL policy constants and `extend_instance_ttl()` helper
4. Schema version constants and migration helpers

These copies have drifted:
- `attester-registry` extends instance TTL in admin/pause operations; `attestation-registry` does not
- `attester-registry` has `upgrade/migrate` functions; `attestation-registry` does not
- Documentation and error text differ between copies
- Every bugfix must be manually ported to both contracts, creating maintenance burden and risk of inconsistency

Soroban's `#[contracttype]` enums serialize by variant position, making variant order ABI-critical. Moving shared logic to a common crate requires a design that does not force a unified `DataKey` enum on both contracts, because that would reorder existing keys and break upgrade safety.

## Decision

Create `crates/lafiya-contract-common` (#![no_std], depending only on soroban-sdk) with three modules:

### Admin Module
- `Admin::get<K: AdminKeys>()` → retrieve current admin
- `Admin::propose<K: AdminKeys>(new_admin)` → propose new admin (current admin auth required)
- `Admin::accept<K: AdminKeys>()` → accept pending transfer (pending admin auth required)
- `Admin::require_auth<K: AdminKeys>()` → enforce admin authorization

### Pausable Module
- `Pausable::is_paused<K: PausableKeys>()` → query pause state
- `Pausable::pause<K: PausableKeys>()` → set paused=true
- `Pausable::unpause<K: PausableKeys>()` → set paused=false
- `Pausable::require_not_paused<K: PausableKeys>()` → revert if paused

### TTL Module
- Constants: `INSTANCE_LIFETIME_THRESHOLD`, `INSTANCE_BUMP_AMOUNT` (canonical values)
- `extend_instance_ttl(env)` → bump instance storage TTL

### Key Design: Trait-Based Key Injection

To preserve per-contract `DataKey` variant order, each module uses a trait:

```rust
pub trait AdminKeys {
    type AdminKey;
    type PendingAdminKey;
    fn admin_key() -> Self::AdminKey;
    fn pending_admin_key() -> Self::PendingAdminKey;
}
```

Each contract implements this trait for its own `DataKey`:

```rust
// In attester-registry
impl AdminKeys for AttesterRegistry {
    type AdminKey = DataKey;
    type PendingAdminKey = DataKey;
    fn admin_key() -> DataKey { DataKey::Admin }
    fn pending_admin_key() -> DataKey { DataKey::PendingAdmin }
}

// Contracts call: Admin::get::<AttesterRegistry>(env)?
```

This approach:
- **Preserves ABI safety:** Each contract's `DataKey` enum variant order remains unchanged
- **No migration required:** Existing stored data continues to decode correctly
- **Zero runtime cost:** Trait resolution is compile-time; no vtable overhead
- **Prevents accidental key collision:** Contracts cannot accidentally mix AdminKeys implementations

### Alternative: DataKey::Common Variant

An alternative would add `DataKey::Common(CommonKey)` as a new final variant in each contract, then migrate admin/pause/ttl data to use these shared keys. This approach:
- **Pros:** Simpler type signatures; no trait constraints
- **Cons:** Requires data migration; adds a storage read/write pass; increases contract complexity; error-prone (easy to miss a key)

Selected the trait approach because it requires zero migration and zero runtime cost.

## Consequences

### Positive

- Single source of truth for admin, pause, and TTL logic
- Automated compilation will catch inconsistencies (all contracts must implement the same trait signatures)
- Bugfixes and improvements only need to be made once
- Easier testing: test helpers can be written in the common crate and reused

### Trade-offs and risks

- Trait overhead in code readability (developers must understand AdminKeys, PausableKeys)
- Generic code can increase WASM binary size if not carefully monomorphized; must measure before/after
- Each contract must implement the trait even if the implementation is trivial
- New contracts must adopt the common crate and implement the traits; cannot accidentally use it

## Follow-up

1. Implement `lafiya-contract-common` crate with Admin, Pausable, TTL modules
2. Update `attester-registry` to depend on and use the common modules
3. Update `attestation-registry` to depend on and use the common modules
4. Measure WASM size before/after; ensure no significant bloat
5. Add integration tests to verify both contracts' pause/admin behavior is identical
6. Remove duplicate code from both contracts
7. Document the trait-based pattern in CONTRIBUTING.md for future contract development

## References

- Issue #363: Extract shared admin, pause, and TTL logic
- Soroban contract upgrade safety: https://stellar.org/docs/build/smart-contracts/storing-data
- Related: ADR-0012 (pause coordination)
