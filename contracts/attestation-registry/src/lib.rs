//! Soroban contract recording attestations of off-chain records, gated by
//! allowlist membership in the `attester-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address,
    BytesN, Env, Symbol, Vec,
};

/// The subset of the `attester-registry` contract this crate calls. Kept
/// as a trait interface (rather than a direct crate dependency) so that
/// `attester-registry`'s own contract implementation never links into this
/// crate's wasm — only the typed cross-contract call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
    fn is_attester_for_region(env: Env, attester: Address, region: Symbol) -> bool;
}

/// Maximum number of historical attestations to keep per record hash.
/// This bounds storage growth per re-attestation. When exceeded,
/// the oldest attestation is removed (FIFO eviction).
const MAX_HISTORY: u64 = 10;

const SCHEMA_VERSION: u32 = 2;

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

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
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// The deployed `attester-registry` contract consulted on every `attest` call.
    AttesterRegistry,
    /// Attestation for a given record hash at a specific sequence number.
    Attestation(BytesN<32>, u64),
    /// Latest sequence number for a given record hash.
    AttestationSequence(BytesN<32>),
    /// Count of attestations for a given record hash (for bounded history).
    AttestationCount(BytesN<32>),
    /// The storage schema version of the contract.
    SchemaVersion,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// Explicit capabilities granted by the owner.
    Role(Role, Address),
}

/// Operational capabilities managed by the owner.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Guardian,
    Revoker,
}

/// A single attestation: proof that `attester` verified the off-chain
/// record whose hash is the lookup key, at `timestamp`. Never contains the
/// underlying health data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    /// The allowlisted attester that verified the record.
    pub attester: Address,
    /// Ledger timestamp at which the attestation was recorded.
    pub timestamp: u64,
}

/// Emitted when admin ownership finishes transferring to a new address.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
}

/// Emitted when a new attestation is recorded for a record hash.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRecorded {
    #[topic]
    pub record_hash: BytesN<32>,
    /// The allowlisted attester that verified the record.
    pub attester: Address,
    /// Ledger timestamp at which the attestation was recorded.
    pub timestamp: u64,
}

/// Emitted when an attestation is revoked.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRevoked {
    #[topic]
    pub record_hash: BytesN<32>,
}

/// Emitted when state-changing operations are paused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Paused {
    #[topic]
    pub by: Address,
}

/// Emitted when state-changing operations are unpaused.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Unpaused {
    #[topic]
    pub by: Address,
}

/// Emitted when the `attester-registry` contract this registry consults is repointed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRegistryRepointed {
    #[topic]
    pub previous: Address,
    #[topic]
    pub new: Address,
}

/// Errors returned by the attestation registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not allowlisted by the `attester-registry` contract.
    AttesterNotAllowlisted = 3,
    /// `accept_admin` was called with no pending admin transfer. Admin transfer is a
    /// two-step flow: the current admin must first call `propose_admin` to nominate a
    /// successor, then the nominated address must call `accept_admin` to complete the
    /// transfer. This error is returned when `accept_admin` is called before a
    /// corresponding `propose_admin` call has set a pending admin.
    NoPendingTransfer = 4,
    /// The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration.
    InvalidRegistryWiring = 5,
    /// No attestation exists for the given record hash / sequence.
    AttestationNotFound = 6,
    /// The requested operation is blocked while the contract is paused.
    ContractPaused = 7,
    /// The supplied address has not been granted the required role.
    RoleNotGranted = 8,
    /// `migrate()` was called when no storage migration is pending.
    MigrationNotRequired = 9,
}

/// Emitted when the contract is upgraded to new wasm.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
}

/// The attestation registry contract.
#[contract]
pub struct AttestationRegistry;

#[contractimpl]
impl AttestationRegistry {
    /// Set the admin and the `attester-registry` contract this registry
    /// consults for allowlist checks. Can only be called once; the caller
    /// must authorize as the given `admin`.
    ///
    /// ## Best-effort interface check
    ///
    /// This function performs a lightweight sanity check against
    /// `attester_registry`: it calls `is_attester_for_region` with a
    /// throwaway address and region, and confirms the call does not trap.
    /// This confirms the address implements the expected interface — it does
    /// **not** prove the address is the canonical, trusted deployment.
    pub fn initialize(env: Env, admin: Address, attester_registry: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        // Best-effort sanity check: verify attester_registry implements the
        // regional allowlist interface with a throwaway address.
        let registry = AttesterRegistryClient::new(&env, &attester_registry);
        // The current contract address is valid but will not be allowlisted.
        let throwaway = env.current_contract_address();
        let throwaway_region = Symbol::new(&env, "interface");
        if registry
            .try_is_attester_for_region(&throwaway, &throwaway_region)
            .is_err()
        {
            return Err(Error::InvalidRegistryWiring);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(())
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Grant a guardian or revoker capability. Only the owner may change roles.
    pub fn grant_role(env: Env, role: Role, account: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Role(role, account.clone()), &true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Revoke a guardian or revoker capability. Only the owner may change roles.
    pub fn revoke_role(env: Env, role: Role, account: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .remove(&DataKey::Role(role, account.clone()));
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Return whether `account` holds `role`.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        env.storage().instance().has(&DataKey::Role(role, account))
    }

    /// Return the configured attester-registry contract address.
    pub fn get_attester_registry(env: Env) -> Result<Address, Error> {
        Self::attester_registry(&env)
    }

    /// Propose a new admin address. The caller must authorize as the current admin.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::admin(&env)?;
        current_admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        Ok(())
    }

    /// Accept the proposed admin transfer. The caller must authorize as the pending admin.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let previous_admin = Self::admin(&env)?;
        let pending_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingTransfer)?;

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);

        AdminTransferred {
            previous_admin,
            new_admin: pending_admin,
        }
        .publish(&env);

        Ok(())
    }

    /// Change the attester-registry contract this registry consults for
    /// allowlist checks. Requires the admin's authorization and a compatible
    /// regional allowlist interface. Emits
    /// `AttesterRegistryRepointed` for indexer/audit visibility.
    pub fn set_attester_registry(env: Env, new_registry: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let registry = AttesterRegistryClient::new(&env, &new_registry);
        let throwaway = env.current_contract_address();
        let throwaway_region = Symbol::new(&env, "interface");
        if registry
            .try_is_attester_for_region(&throwaway, &throwaway_region)
            .is_err()
        {
            return Err(Error::InvalidRegistryWiring);
        }

        let previous = Self::attester_registry(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &new_registry);

        AttesterRegistryRepointed {
            previous,
            new: new_registry,
        }
        .publish(&env);

        Ok(())
    }

    /// Pause the contract, blocking `attest` until `unpause` is called.
    /// Requires the Guardian role.
    pub fn pause(env: Env, guardian: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Guardian, &guardian)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: guardian }.publish(&env);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the owner's authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused { by: admin }.publish(&env);
        Ok(())
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Record that `attester` verified the record hashing to `record_hash` in
    /// `region`.
    /// Requires `attester`'s authorization and that `attester` is
    /// currently allowlisted in the configured `attester-registry`.
    /// Stores the attestation with an incrementing sequence number,
    /// maintaining a bounded history (MAX_HISTORY entries per hash).
    pub fn attest(
        env: Env,
        attester: Address,
        record_hash: BytesN<32>,
        region: Symbol,
    ) -> Result<Attestation, Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;

        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester_for_region(&attester, &region) {
            return Err(Error::AttesterNotAllowlisted);
        }

        let attestation = Attestation {
            attester: attester.clone(),
            timestamp: env.ledger().timestamp(),
        };

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .unwrap_or(0);
        let new_sequence = sequence + 1;

        env.storage().persistent().set(
            &DataKey::Attestation(record_hash.clone(), new_sequence),
            &attestation,
        );

        env.storage().persistent().set(
            &DataKey::AttestationSequence(record_hash.clone()),
            &new_sequence,
        );

        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);
        let new_count = count + 1;

        if new_count > MAX_HISTORY {
            let oldest_sequence = new_count.saturating_sub(MAX_HISTORY);
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), oldest_sequence));
        }

        env.storage()
            .persistent()
            .set(&DataKey::AttestationCount(record_hash.clone()), &new_count);

        // Extend TTL on the specific attestation entry just written, so it is
        // not subject to state-archival independently of the instance storage.
        env.storage().persistent().extend_ttl(
            &DataKey::Attestation(record_hash.clone(), new_sequence),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AttestationRecorded {
            record_hash,
            attester,
            timestamp: attestation.timestamp,
        }
        .publish(&env);

        Ok(attestation)
    }

    /// Revoke all attestations for `record_hash`. Requires the Revoker role.
    pub fn revoke_attestation(
        env: Env,
        revoker: Address,
        record_hash: BytesN<32>,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Revoker, &revoker)?;

        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
            .ok_or(Error::NotInitialized)?;

        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);

        let start_sequence = if count > MAX_HISTORY {
            sequence.saturating_sub(MAX_HISTORY - 1)
        } else {
            1
        };

        for seq in start_sequence..=sequence {
            env.storage()
                .persistent()
                .remove(&DataKey::Attestation(record_hash.clone(), seq));
        }
        env.storage()
            .persistent()
            .remove(&DataKey::AttestationSequence(record_hash.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::AttestationCount(record_hash.clone()));

        AttestationRevoked { record_hash }.publish(&env);

        Ok(())
    }

    /// Look up the latest attestation for `record_hash`, if any. Callable
    /// by anyone — this is what lets a responder's QR scan independently
    /// check a card without an external oracle.
    pub fn get_attestation(env: Env, record_hash: BytesN<32>) -> Option<Attestation> {
        let sequence: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))?;
        env.storage()
            .persistent()
            .get(&DataKey::Attestation(record_hash, sequence))
    }

    /// Look up the full attestation history for `record_hash`, if any.
    /// Returns attestations in chronological order (oldest first).
    /// Callable by anyone.
    pub fn get_attestation_history(env: Env, record_hash: BytesN<32>) -> Vec<Attestation> {
        let sequence: u64 = match env
            .storage()
            .persistent()
            .get(&DataKey::AttestationSequence(record_hash.clone()))
        {
            Some(seq) => seq,
            None => return Vec::new(&env),
        };

        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationCount(record_hash.clone()))
            .unwrap_or(0);

        let mut history = Vec::new(&env);
        let start_sequence = if count > MAX_HISTORY {
            sequence.saturating_sub(MAX_HISTORY - 1)
        } else {
            1
        };

        for seq in start_sequence..=sequence {
            if let Some(attestation) = env
                .storage()
                .persistent()
                .get(&DataKey::Attestation(record_hash.clone(), seq))
            {
                history.push_back(attestation);
            }
        }

        history
    }

    /// Upgrade the contract's Wasm code. Only the owner may authorize upgrades.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        Upgraded { new_wasm_hash }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Complete a pending storage migration after an upgrade.
    pub fn migrate(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let stored: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(1);
        if stored >= SCHEMA_VERSION {
            return Err(Error::MigrationNotRequired);
        }

        // The owner remains stored at the legacy Admin key. Role entries are
        // additive and remain empty until explicitly granted.
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn require_role(env: &Env, role: Role, account: &Address) -> Result<(), Error> {
        if !env
            .storage()
            .instance()
            .has(&DataKey::Role(role, account.clone()))
        {
            return Err(Error::RoleNotGranted);
        }
        account.require_auth();
        Ok(())
    }

    fn attester_registry(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)
    }

    fn require_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod fuzz_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
