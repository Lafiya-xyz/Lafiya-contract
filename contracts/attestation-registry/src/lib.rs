#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address,
    BytesN, Env,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
}

/// Storage schema version implemented by this build of the contract.
///
/// Bump this ONLY when a release changes the shape or meaning of data in
/// storage (stored struct fields, key layout, enum discriminants). Such a
/// release must extend `migrate()` with the step that moves stored data from
/// version `SCHEMA_VERSION - 1` to `SCHEMA_VERSION`, oldest step first. See
/// `docs/runbooks/contract-upgrade.md`.
const SCHEMA_VERSION: u32 = 1;

/// Storage keys for the attestation registry.
///
/// UPGRADE SAFETY: `#[contracttype]` enums serialize variants by their
/// position index, so variant order and existing variants must never change
/// — append new variants at the end only. Reordering breaks decoding of
/// data written by earlier versions.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to (re)point `AttesterRegistry` and to upgrade
    /// the contract.
    Admin,
    /// The deployed `attester-registry` contract consulted on every `attest` call.
    AttesterRegistry,
    /// Latest attestation recorded for a given record hash.
    Attestation(BytesN<32>),
    /// Storage schema version recorded for this instance (set by
    /// `initialize`/`migrate`).
    SchemaVersion,
}

/// A single attestation: proof that `attester` verified the off-chain
/// record whose hash is the lookup key, at `timestamp`. Never contains the
/// underlying health data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    pub attester: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRecorded {
    #[topic]
    pub record_hash: BytesN<32>,
    pub attester: Address,
    pub timestamp: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    AttesterNotAllowlisted = 3,
    MigrationNotRequired = 4,
}

#[contract]
pub struct AttestationRegistry;

#[contractimpl]
impl AttestationRegistry {
    /// Set the admin and the `attester-registry` contract this registry
    /// consults for allowlist checks. Can only be called once; the caller
    /// must authorize as the given `admin`.
    pub fn initialize(env: Env, admin: Address, attester_registry: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(())
    }

    /// Record that `attester` verified the record hashing to `record_hash`.
    /// Requires `attester`'s authorization and that `attester` is
    /// currently allowlisted in the configured `attester-registry`.
    /// Overwrites any prior attestation for the same `record_hash`.
    pub fn attest(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
    ) -> Result<Attestation, Error> {
        attester.require_auth();

        let registry_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

        let attestation = Attestation {
            attester,
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attestation(record_hash.clone()), &attestation);

        AttestationRecorded {
            record_hash,
            attester: attestation.attester.clone(),
            timestamp: attestation.timestamp,
        }
        .publish(&env);

        Ok(attestation)
    }

    /// Look up the latest attestation for `record_hash`, if any. Callable
    /// by anyone — this is what lets a responder's QR scan independently
    /// check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        env.storage()
            .persistent()
            .get(&DataKey::Attestation(record_hash))
    }

    /// The storage schema version recorded for this instance. `0` means no
    /// version has been recorded: the instance was deployed before schema
    /// versioning landed (legacy) or was never initialized.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(0)
    }

    /// Replace this contract's code with the wasm blob identified by
    /// `new_wasm_hash`. The blob must already have been uploaded to the
    /// ledger (e.g. `stellar contract upload`); otherwise the ledger rejects
    /// the update. Requires the admin's authorization. Instance and
    /// persistent storage are untouched — the new code starts exactly where
    /// the old code left off. The swap itself takes effect once this
    /// invocation finishes successfully. See
    /// `docs/runbooks/contract-upgrade.md` for the full production
    /// procedure, including how reviewers verify `new_wasm_hash` against
    /// the audited source before this call is signed.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    /// Run any pending storage migration, then record the new schema
    /// version. Requires the admin's authorization.
    ///
    /// Call this after `upgrade()` only when the new build bumps
    /// `SCHEMA_VERSION` (a storage-schema-changing release) — including the
    /// first upgrade of a legacy (pre-versioning, schema version `0`)
    /// instance, which must be migrated to version 1. When no migration is
    /// pending (`SchemaVersion >= SCHEMA_VERSION`) this returns
    /// `Error::MigrationNotRequired` so the call can't accidentally re-run.
    pub fn migrate(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();

        let stored = Self::get_schema_version(env.clone());
        if stored >= SCHEMA_VERSION {
            return Err(Error::MigrationNotRequired);
        }

        // Per-version migration steps, oldest first. This build introduces
        // schema version 1, whose layout is identical to the legacy
        // (unversioned) layout, so no data reshaping is required here.
        // Schema-changing releases insert their steps below, guarded by the
        // version they migrate FROM, e.g.:
        //
        //   if stored < 2 { /* move/reshape v1 data into the v2 layout */ }
        //   if stored < 3 { /* ... */ }

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
