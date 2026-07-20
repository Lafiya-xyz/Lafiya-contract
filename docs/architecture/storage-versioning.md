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
3. **Initialization**: During `initialize()`, the contract sets `SchemaVersion` in instance storage:
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
