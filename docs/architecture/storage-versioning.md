# Storage Versioning and Upgrade Conventions

This document defines the architecture and conventions for managing storage schema changes and versioning across contract upgrades in Lafiya's Soroban smart contracts.

## 1. DataKey Evolution

Soroban contract storage relies on keys serialized via XDR. For `DataKey` enums, we enforce the following rules to prevent key collision or deserialization failure when contract bytecode is upgraded:

### Additive-Only Variants
* **Rule**: New keys/variants must be added to the end of the `DataKey` enum.
* **Rule**: Existing variants must **never** be reordered, deleted, or have their types modified.
* **Reason**: Soroban serializes enums based on their variant discriminants (index order). Changing the order or deleting a variant will shift the discriminants of subsequent variants, leading to silent collisions or failure to deserialize previously stored keys.

### Explicit Namespace Segmentation
* If a variant stores dynamic parameters (e.g., `Attestation(BytesN<32>)`), ensure the inner types are fully versioned or structured if their fields are subject to change.

---

## 2. The `SchemaVersion` Key

Every contract must maintain an explicit schema version in its instance storage.

### Implementation Details
1. **Schema Version Variant**: A `SchemaVersion` variant is included in the `DataKey` enum.
2. **Current Version Constant**: A `const SCHEMA_VERSION: u32` defines the schema version supported by the current bytecode.
3. **Construction**: The deployment constructor sets `SchemaVersion` in instance storage:
   ```rust
   env.storage().instance().set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
   ```
4. **View Interface**: Every contract must expose a read-only view function to query the current version:
   ```rust
   pub fn get_schema_version(env: Env) -> u32 {
       env.storage().instance().get(&DataKey::SchemaVersion).unwrap_or(1)
   }
   ```
   *Note: Defaulting to 1 ensures backwards compatibility with contracts deployed before schema versioning was introduced.*

---

## 3. Contract Upgrade and Migration Pattern

When upgrading a contract to a new bytecode version with schema changes, a migration pattern must be followed.

### The `upgrade()` Entrypoint
Upgradable contracts should expose an `upgrade(env: Env, new_wasm_hash: BytesN<32>)` function restricted to the `Admin`.

### Migration Flow
Within the upgrade or post-upgrade logic:
1. Read the old version from instance storage:
   ```rust
   let old_version: u32 = env.storage().instance().get(&DataKey::SchemaVersion).unwrap_or(1);
   ```
2. If `old_version < SCHEMA_VERSION`, execute conditional migration logic:
   ```rust
   if old_version < 2 {
       // Run migration from version 1 to 2
       migrate_v1_to_v2(&env);
   }
   ```
3. Update the stored schema version to the new `SCHEMA_VERSION`:
   ```rust
   env.storage().instance().set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
   ```

### Writing Migration Functions
* Keep migrations safe by using `instance` or `temporary` storage where appropriate.
* For `persistent` storage migration (which can be large and exceed budget limits), prefer lazy/on-demand migration during normal reads/writes, or paginated/chunked migrations triggered by administrative calls.

---

## 4. Time Abstraction: `Seconds` vs `Ledgers`

Soroban exposes two independent time bases, and mixing them silently is a source of subtle bugs:

* **Ledger timestamps** — a Unix timestamp in **seconds**, advanced by the network's ledger close time.
* **Ledger sequences** — a monotonically increasing **ledger count**, advanced once per closed ledger.

The conversion between the two depends on the ledger close time, which is **not** a protocol constant: it has changed across upgrades (historically ~5s, now ~5s with a target of faster closes). A "7-day" window expressed as a fixed ledger count therefore silently becomes 5 or 9 days when the close time changes.

### Typed Newtypes

All time values in contract code must use the typed newtypes from `lafiya_contract_common::time`:

```rust
pub struct Seconds(pub u64);
pub struct Ledgers(pub u32);
```

Raw `u32` / `u64` values must never be used to represent a duration, timestamp, or ledger count. This makes the time base explicit at every call site and prevents accidental cross-base arithmetic.

### Declared Close-Time Assumption

Conversions between `Seconds` and `Ledgers` are **explicit** and require a declared close-time assumption:

```rust
/// Assumed ledger close time used for all Seconds <-> Ledgers conversions.
pub const ASSUMED_CLOSE_TIME_SECONDS: u32 = 5;

/// Safety factor applied when converting a validity window into a TTL.
/// A TTL must cover the validity window even if ledgers close 50% faster.
pub const TTL_SAFETY_FACTOR_NUM: u32 = 3;
pub const TTL_SAFETY_FACTOR_DEN: u32 = 2;

impl Seconds {
    /// Convert to a ledger count assuming `ASSUMED_CLOSE_TIME_SECONDS`,
    /// rounded up so the result never under-covers the requested duration.
    pub fn to_ledgers(self) -> Ledgers { /* ceil(self.0 / close_time) */ }
}

impl Ledgers {
    /// Convert to seconds assuming `ASSUMED_CLOSE_TIME_SECONDS`.
    pub fn to_seconds(self) -> Seconds { /* self.0 * close_time */ }
}
```

Rules:

* Conversions are **always** explicit function calls — never implicit `as` casts.
* Conversions round **up** when producing `Ledgers` from `Seconds`, so a TTL never under-covers its validity window.
* The assumption is declared in one place (`ASSUMED_CLOSE_TIME_SECONDS`) and documented next to every constant derived from it.

### TTL-Covers-Validity Rule

Every time-bounded storage entry (consent records, proposals, rate-limit windows, payout delays) must have a TTL that covers its validity window **even if ledgers close 50% faster than assumed**:

```
ttl_ledgers >= ceil(validity_seconds / (ASSUMED_CLOSE_TIME_SECONDS / 1.5))
```

Equivalently, using the safety factor above:

```
ttl_ledgers >= validity_seconds.to_ledgers() * TTL_SAFETY_FACTOR_NUM / TTL_SAFETY_FACTOR_DEN
```

Each time-bounded entry must ship a test asserting this inequality, e.g.:

```rust
#[test]
fn consent_ttl_covers_validity_at_faster_close_time() {
    let faster_close_time = ASSUMED_CLOSE_TIME_SECONDS * 2 / 3; // 50% faster
    let required = CONSENT_VALIDITY_SECONDS.0.div_ceil(faster_close_time as u64);
    assert!(CONSENT_TTL_THRESHOLD.0 as u64 >= required);
}
```

This guarantees that a "7-day" consent remains readable for at least 7 days of wall-clock time regardless of protocol close-time changes.

### Lint: No Raw Time Constants

Contract code must not contain raw `u32` / `u64` time constants. A CI check (`scripts/check-time-constants.sh`, wired into the `lint` job) greps contract sources for numeric literals assigned to identifiers matching `*_SECONDS`, `*_LEDGERS`, `*_TTL*`, `*_WINDOW*`, or `*_DELAY*` and fails the build unless the value is wrapped in `Seconds(...)` or `Ledgers(...)`.

```rust
// ❌ Rejected by lint
const CONSENT_VALIDITY_SECONDS: u64 = 604_800;

// ✅ Accepted
const CONSENT_VALIDITY: Seconds = Seconds(604_800);
const CONSENT_TTL_THRESHOLD: Ledgers = Ledgers(/* derived via to_ledgers() */);
```

When adding a new time-bounded entry:

1. Declare its validity as a `Seconds` constant.
2. Derive its TTL as a `Ledgers` constant using the safety factor.
3. Add a TTL-covers-validity test.
4. Document the close-time assumption if it deviates from `ASSUMED_CLOSE_TIME_SECONDS`.
