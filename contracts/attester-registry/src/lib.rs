//! Soroban contract maintaining the allowlist of attesters authorized to
//! call `attest` on the `attestation-registry` contract.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    Symbol, Vec,
};

const SCHEMA_VERSION: u32 = 1;
const ADMIN_PROPOSAL_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Interface kind reported by `get_interface`, used by clients as a weak
/// identity check when wiring contracts (not proof of authenticity).
pub const CONTRACT_KIND: &str = "lafiya_attester_registry";

/// Version of the public contract interface (functions, errors, types).
/// Bump on any breaking ABI change; `scripts/conformance/check_snapshot.py`
/// refuses a breaking snapshot update without a bump.
pub const INTERFACE_VERSION: u32 = 1;

/// Version of the emitted event schemas (see `docs/events.md`).
pub const EVENT_VERSION: u32 = 1;

/// Optional features this build supports, reported by `get_interface`.
pub const FEATURES: [&str; 6] = [
    "pause",
    "metadata",
    "suspension",
    "batch",
    "max_attesters",
    "migrate",
];

/// Interface and capability metadata returned by `get_interface`, so
/// clients can negotiate features with one call instead of probing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceInfo {
    /// Contract kind, e.g. `lafiya_attester_registry`.
    pub contract_kind: Symbol,
    /// Public interface version; bumped on any breaking ABI change.
    pub interface_version: u32,
    /// Optional features enabled in this build.
    pub features: Vec<Symbol>,
    /// Storage schema version (see `get_schema_version`).
    pub schema_version: u32,
    /// Event schema version.
    pub event_version: u32,
}

/// Storage keys for the attester registry.
///
/// UPGRADE SAFETY: `#[contracttype]` enums serialize variants by their
/// position index, so variant order and existing variants must never change
/// — append new variants at the end only. Reordering breaks decoding of
/// data written by earlier versions.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to add/remove attesters and to upgrade the
    /// contract.
    Admin,
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// Presence of this key (mapped to `AttesterInfo`) means the address is an
    /// allowlisted attester.
    Attester(Address),
    /// Presence of this key means the attester is currently suspended.
    Suspended(Address),
    /// The storage schema version of the contract.
    SchemaVersion,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// Soft cap on the number of allowlisted attesters.
    MaxAttesters,
    /// Current count of allowlisted attesters.
    AttesterCount,
    /// Ledger timestamp at which the pending admin proposal expires.
    PendingAdminExpiresAt,
    /// The latest successor address for a rotated attester.
    AttesterRotation(Address),
    /// A point-in-time attester status transition.
    AttesterStatusChange(Address, u32),
    /// Number of recorded status transitions for an attester.
    AttesterStatusChangeCount(Address),
}

/// Emitted when an attester rotates its key while retaining its enrollment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRotated {
    #[topic]
    pub previous_attester: Address,
    #[topic]
    pub new_attester: Address,
}

/// Metadata associated with an allowlisted attester.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterInfo {
    /// Hash of the attester's off-chain license/credential document, if any.
    pub license_hash: Option<BytesN<32>>,
    /// The geographic region the attester is authorized to attest for, if any.
    pub region: Option<Symbol>,
}

/// A status transition retained for point-in-time allowlist queries.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterStatusChange {
    pub timestamp: u64,
    pub active: bool,
}

/// An allowlisted attester's metadata together with its current suspension
/// state, as returned by `get_attester_status`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttesterStatus {
    /// The attester's stored metadata.
    pub info: AttesterInfo,
    /// Whether the attester is currently suspended.
    pub suspended: bool,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Default soft cap on the number of allowlisted attesters, used until an
/// admin raises it via `set_max_attesters`. Sized generously above any
/// realistic CHW allowlist so it never trips in normal operation; it exists
/// so a compromised or buggy admin key can't grow persistent-storage rent
/// unboundedly.
const DEFAULT_MAX_ATTESTERS: u32 = 50_000;

/// Maximum number of addresses that may be processed in a single
/// `add_attesters` / `remove_attesters` call.
///
/// Rationale: each address in the batch is one persistent-storage write entry.
/// Soroban's per-transaction write-entry limit is 50, so a ceiling of 40 gives
/// headroom for the instance-storage writes (AttesterCount, Paused, etc.) that
/// happen in the same transaction. Batches larger than this are rejected with
/// `Error::BatchTooLarge` — an early, deterministic error rather than a silent
/// resource-limit abort at the network layer.
pub const BATCH_LIMIT: u32 = 40;

/// Errors returned by the attester registry's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` has not been called yet; call
    /// `initialize(admin: Address)` before using the contract.
    /// @severity operator
    NotInitialized = 1,
    /// `initialize` was called more than once.
    /// @severity operator
    AlreadyInitialized = 2,
    /// `accept_admin` was called with no pending admin transfer. Admin transfer is a
    /// two-step flow: the current admin must first call `propose_admin` to nominate a
    /// successor, then the nominated address must call `accept_admin` to complete the
    /// transfer. This error is returned when `accept_admin` is called before a
    /// corresponding `propose_admin` call has set a pending admin.
    NoPendingTransfer = 3,
    /// The requested operation is blocked while the contract is paused.
    ContractPaused = 4,
    /// The allowlist is at its configured maximum size. Raise the cap via `set_max_attesters`, or free a slot via `remove_attester`.
    AllowlistFull = 5,
    /// `migrate()` was called while the stored schema version is already
    /// `>= SCHEMA_VERSION`. Only call `migrate()` after `upgrade()` to a
    /// build that bumps `SCHEMA_VERSION`; this error is a safe no-op signal
    /// that there is nothing pending, not a failure to react to.
    /// @severity operator
    MigrationNotRequired = 6,
    /// The referenced attester is not currently allowlisted (never added,
    /// or since removed).
    AttesterNotFound = 7,
    /// The supplied batch exceeds `BATCH_LIMIT` addresses.
    BatchTooLarge = 8,
    /// The proposed admin address is not a valid successor.
    InvalidAdminProposal = 9,
    /// The pending admin proposal has expired.
    ProposalExpired = 10,
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

/// Emitted when the current admin nominates a successor.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferProposed {
    #[topic]
    pub current_admin: Address,
    #[topic]
    pub proposed_admin: Address,
    pub expires_at: u64,
}

/// Emitted when a pending admin transfer is cancelled.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferCancelled {
    #[topic]
    pub admin: Address,
    #[topic]
    pub proposed_admin: Address,
}

/// Emitted once, when the contract is initialized.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
}

/// Emitted when an attester is added to the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterAdded {
    #[topic]
    pub attester: Address,
}

/// Emitted when an already-allowlisted attester's metadata is updated via
/// `update_attester_info`. Distinguishable from `AttesterAdded`, which is
/// only emitted on initial enrollment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterInfoUpdated {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester is removed from the allowlist.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRemoved {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester revokes its own key.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRevoked {
    #[topic]
    pub attester: Address,
}

/// Emitted when an attester is suspended.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterSuspended {
    #[topic]
    pub attester: Address,
}

/// Emitted when a suspended attester is reinstated.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterReinstated {
    #[topic]
    pub attester: Address,
}

/// Emitted when the contract is upgraded to new wasm.
#[contractevent]
#[derive(Clone, Debug)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
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

// Source-provenance metadata (SEP-46 `contractmetav0`, keys per SEP-55
// "Contract Build Verification"). Deterministic for a given commit:
// LAFIYA_GIT_COMMIT is injected by build.rs, see docs/releasing.md.
soroban_sdk::contractmeta!(
    key = "source_repo",
    val = "github:Lafiya-xyz/Lafiya-contract"
);
soroban_sdk::contractmeta!(key = "home_domain", val = "lafiya-xyz.github.io");
soroban_sdk::contractmeta!(key = "crate_name", val = env!("CARGO_PKG_NAME"));
soroban_sdk::contractmeta!(key = "crate_version", val = env!("CARGO_PKG_VERSION"));
soroban_sdk::contractmeta!(key = "source_rev", val = env!("LAFIYA_GIT_COMMIT"));

/// The attester registry contract.
#[contract]
pub struct AttesterRegistry;

#[contractimpl]
impl AttesterRegistry {
    /// Set the admin address authorized to manage the allowlist. Can only
    /// be called once; the caller must authorize as the given `admin`.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Rotate an allowlisted attester key without losing metadata or enrollment history.
    /// Both the old and new addresses must authorize the operation.
    pub fn rotate_attester(
        env: Env,
        previous_attester: Address,
        new_attester: Address,
    ) -> Result<(), Error> {
        previous_attester.require_auth();
        new_attester.require_auth();
        Self::require_not_paused(&env)?;
        if previous_attester == new_attester
            || !env
                .storage()
                .persistent()
                .has(&DataKey::Attester(previous_attester.clone()))
            || env
                .storage()
                .persistent()
                .has(&DataKey::Attester(new_attester.clone()))
        {
            return Err(Error::AttesterNotFound);
        }

        let info: AttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(previous_attester.clone()))
            .ok_or(Error::AttesterNotFound)?;
        let suspended = env
            .storage()
            .persistent()
            .has(&DataKey::Suspended(previous_attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Attester(previous_attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(previous_attester.clone()));
        env.storage()
            .persistent()
            .set(&DataKey::Attester(new_attester.clone()), &info);
        if suspended {
            env.storage()
                .persistent()
                .set(&DataKey::Suspended(new_attester.clone()), &true);
        }
        env.storage().persistent().set(
            &DataKey::AttesterRotation(previous_attester.clone()),
            &new_attester,
        );
        Self::record_status_change(&env, &previous_attester, false);
        Self::record_status_change(&env, &new_attester, !suspended);
        AttesterRotated {
            previous_attester,
            new_attester,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Revoke the caller's own attester key immediately. This remains available
    /// while paused so a compromised key can be stopped without admin action.
    pub fn revoke_attester(env: Env, attester: Address) -> Result<(), Error> {
        attester.require_auth();
        let was_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
        if was_present {
            env.storage()
                .persistent()
                .remove(&DataKey::Attester(attester.clone()));
            env.storage()
                .persistent()
                .remove(&DataKey::Suspended(attester.clone()));
            let count = Self::attester_count(&env);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::AttesterCount, &(count - 1));
            }
            Self::record_status_change(&env, &attester, false);
        }
        AttesterRevoked { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Return the latest address to which `attester` was rotated, if any.
    pub fn get_attester_rotation(env: Env, attester: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::AttesterRotation(attester))
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Propose a new admin address. The caller must authorize as the current admin.
    /// Calling this a second time before `accept_admin` overwrites any pending proposal — the most recent call wins.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::admin(&env)?;
        current_admin.require_auth();
        if new_admin == current_admin || new_admin == env.current_contract_address() {
            return Err(Error::InvalidAdminProposal);
        }
        let expires_at = env
            .ledger()
            .timestamp()
            .saturating_add(ADMIN_PROPOSAL_TTL_SECONDS);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdminExpiresAt, &expires_at);
        AdminTransferProposed {
            current_admin,
            proposed_admin: new_admin,
            expires_at,
        }
        .publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
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
        let expires_at: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdminExpiresAt)
            .unwrap_or(0);
        if env.ledger().timestamp() > expires_at {
            env.storage().instance().remove(&DataKey::PendingAdmin);
            env.storage()
                .instance()
                .remove(&DataKey::PendingAdminExpiresAt);
            return Err(Error::ProposalExpired);
        }

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.storage()
            .instance()
            .remove(&DataKey::PendingAdminExpiresAt);

        AdminTransferred {
            previous_admin,
            new_admin: pending_admin,
        }
        .publish(&env);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Cancel the pending admin transfer. Requires the current admin's authorization.
    pub fn cancel_admin_proposal(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        let proposed_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingTransfer)?;
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.storage()
            .instance()
            .remove(&DataKey::PendingAdminExpiresAt);
        AdminTransferCancelled {
            admin,
            proposed_admin,
        }
        .publish(&env);
        Ok(())
    }

    /// Pause the contract, blocking `add_attester`, `add_attester_with_info`,
    /// `update_attester_info`, `remove_attester`, `suspend_attester`, and
    /// `reinstate_attester` until `unpause` is called. Requires the admin's
    /// authorization.
    pub fn pause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: admin }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires the admin's authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused { by: admin }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Add `attester` to the allowlist. Requires the admin's authorization.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`).
    pub fn add_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        let already_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
        if !already_present {
            let count = Self::attester_count(&env);
            let max = Self::max_attesters(&env);
            if count >= max {
                return Err(Error::AllowlistFull);
            }
            env.storage()
                .instance()
                .set(&DataKey::AttesterCount, &(count + 1));
        }
        let info = AttesterInfo {
            license_hash: None,
            region: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
    /// Fails with `Error::AllowlistFull` if the allowlist is at capacity and
    /// `attester` is not already present (see `set_max_attesters`).
    pub fn add_attester_with_info(
        env: Env,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        let already_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
        if !already_present {
            let count = Self::attester_count(&env);
            let max = Self::max_attesters(&env);
            if count >= max {
                return Err(Error::AllowlistFull);
            }
            env.storage()
                .instance()
                .set(&DataKey::AttesterCount, &(count + 1));
        }
        let info = AttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        Self::record_status_change(&env, &attester, true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterAdded { attester }.publish(&env);
        Ok(())
    }

    /// Update the metadata of an already-allowlisted `attester`. Requires
    /// the admin's authorization. Unlike `add_attester_with_info`, this
    /// never enrolls a new attester: it fails with `Error::AttesterNotFound`
    /// if `attester` is not currently allowlisted (never added, or since
    /// removed), and always emits `AttesterInfoUpdated` rather than
    /// `AttesterAdded`, so profile changes are distinguishable from
    /// enrollment.
    pub fn update_attester_info(
        env: Env,
        attester: Address,
        license_hash: Option<BytesN<32>>,
        region: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()))
        {
            return Err(Error::AttesterNotFound);
        }
        let info = AttesterInfo {
            license_hash,
            region,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Attester(attester.clone()), &info);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        AttesterInfoUpdated { attester }.publish(&env);
        Ok(())
    }

    /// Add multiple attesters to the allowlist in a single transaction.
    ///
    /// Requires the admin's authorization. Blocked while the contract is paused.
    /// Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
    /// Returns `Error::AllowlistFull` if adding the new (non-duplicate)
    /// addresses would exceed the configured `max_attesters` cap. Addresses
    /// that are already allowlisted are silently skipped (idempotent), so the
    /// call never fails due to duplicates in the batch and no duplicate events
    /// are emitted. Exactly one `AttesterAdded` event is emitted per newly
    /// added address.
    pub fn add_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        let max = Self::max_attesters(&env);
        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            if !env.storage().persistent().has(&key) {
                if count >= max {
                    return Err(Error::AllowlistFull);
                }
                let info = AttesterInfo {
                    license_hash: None,
                    region: None,
                };
                env.storage().persistent().set(&key, &info);
                Self::record_status_change(&env, &attester, true);
                count += 1;
                AttesterAdded {
                    attester: attester.clone(),
                }
                .publish(&env);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::AttesterCount, &count);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Remove multiple attesters from the allowlist in a single transaction.
    ///
    /// Requires the admin's authorization. Blocked while the contract is paused.
    /// Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
    /// Addresses that are not currently allowlisted are silently skipped
    /// (idempotent), so the call never fails if an address was already removed
    /// and no spurious events are emitted. Exactly one `AttesterRemoved` event
    /// is emitted per address that was actually removed.
    pub fn remove_attesters(env: Env, attesters: Vec<Address>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if attesters.len() > BATCH_LIMIT {
            return Err(Error::BatchTooLarge);
        }

        let mut count = Self::attester_count(&env);

        for attester in attesters.iter() {
            let key = DataKey::Attester(attester.clone());
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
                env.storage()
                    .persistent()
                    .remove(&DataKey::Suspended(attester.clone()));
                if count > 0 {
                    count -= 1;
                }
                Self::record_status_change(&env, &attester, false);
                AttesterRemoved {
                    attester: attester.clone(),
                }
                .publish(&env);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::AttesterCount, &count);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Remove `attester` from the allowlist. Requires the admin's
    /// authorization. A no-op if the attester was never allowlisted.
    pub fn remove_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        let was_present = env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Attester(attester.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        if was_present {
            let count = Self::attester_count(&env);
            if count > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::AttesterCount, &(count - 1));
            }
            Self::record_status_change(&env, &attester, false);
        }
        AttesterRemoved { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Set the soft cap on the number of allowlisted attesters. Requires the
    /// admin's authorization. Does not evict existing attesters if lowered
    /// below the current count; it only blocks further `add_attester` calls.
    pub fn set_max_attesters(env: Env, max_attesters: u32) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::MaxAttesters, &max_attesters);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// The current soft cap on the number of allowlisted attesters.
    pub fn get_max_attesters(env: Env) -> u32 {
        Self::max_attesters(&env)
    }

    /// The current number of allowlisted attesters.
    pub fn get_attester_count(env: Env) -> u32 {
        Self::attester_count(&env)
    }

    /// Suspend an allowlisted attester. Requires the admin's authorization.
    ///
    /// **Note:** this function does **not** check whether `attester` was ever
    /// added via `add_attester`. If called on an address that is not in the
    /// allowlist, it silently sets the `Suspended` storage key and emits
    /// `AttesterSuspended` for that address — a no-op from an access-control
    /// perspective because `is_attester` also checks for an `Attester` storage
    /// entry, so the phantom suspension has no effect on allowlist queries.
    /// This diverges from `update_attester_info`, which returns
    /// `Error::AttesterNotFound` for unknown addresses. The inconsistency is
    /// known and documented here rather than silently changed; a follow-up
    /// issue should decide whether to align both functions.
    pub fn suspend_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::Suspended(attester.clone()), &true);
        Self::record_status_change(&env, &attester, false);
        AttesterSuspended { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Reinstate a suspended attester. Requires the admin's authorization.
    pub fn reinstate_attester(env: Env, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;
        env.storage()
            .persistent()
            .remove(&DataKey::Suspended(attester.clone()));
        AttesterReinstated { attester }.publish(&env);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Whether `attester` is currently allowlisted (and not suspended). Callable by anyone,
    /// including other contracts (e.g. `attestation-registry`).
    pub fn is_attester(env: Env, attester: Address) -> bool {
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Attester(attester.clone()))
        {
            return false;
        }
        !env.storage()
            .persistent()
            .has(&DataKey::Suspended(attester))
    }

    /// Whether `attester` was active at the supplied ledger timestamp.
    pub fn is_attester_at(env: Env, attester: Address, timestamp: u64) -> bool {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AttesterStatusChangeCount(attester.clone()))
            .unwrap_or(0);
        let mut active = false;
        for sequence in 1..=count {
            let change: Option<AttesterStatusChange> = env
                .storage()
                .persistent()
                .get(&DataKey::AttesterStatusChange(attester.clone(), sequence));
            if let Some(change) = change {
                if change.timestamp > timestamp {
                    break;
                }
                active = change.active;
            }
        }
        active
    }

    /// Get the optional metadata associated with `attester` if they are allowlisted.
    pub fn get_attester_info(env: Env, attester: Address) -> Option<AttesterInfo> {
        env.storage().persistent().get(&DataKey::Attester(attester))
    }

    /// Get `attester`'s metadata together with its current suspension state
    /// in a single call. Returns `None` if `attester` is not currently
    /// allowlisted (never added, or since removed).
    pub fn get_attester_status(env: Env, attester: Address) -> Option<AttesterStatus> {
        let info: AttesterInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Attester(attester.clone()))?;
        let suspended = env
            .storage()
            .persistent()
            .has(&DataKey::Suspended(attester));
        Some(AttesterStatus { info, suspended })
    }

    /// Report the contract kind, interface version, enabled features, and
    /// storage/event schema versions for runtime compatibility negotiation.
    /// Callable by anyone.
    pub fn get_interface(env: Env) -> InterfaceInfo {
        let mut features = Vec::new(&env);
        for feature in FEATURES {
            features.push_back(Symbol::new(&env, feature));
        }
        InterfaceInfo {
            contract_kind: Symbol::new(&env, CONTRACT_KIND),
            interface_version: INTERFACE_VERSION,
            features,
            schema_version: Self::get_schema_version(env.clone()),
            event_version: EVENT_VERSION,
        }
    }

    /// Query the current storage schema version of the contract.
    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(1)
    }

    /// Upgrade the contract's Wasm code to a new version.
    /// Requires the admin's authorization.
    ///
    /// Runbook:
    /// 1. Build the new Wasm binary (e.g. `cargo build --workspace --release --target wasm32v1-none`).
    /// 2. Upload/install the new Wasm on-chain to obtain its 32-byte hash (`new_wasm_hash`).
    /// 3. The admin calls this `upgrade` function passing the `new_wasm_hash`.
    ///
    /// For any accompanying state/data migrations, see the storage-versioning guidelines
    /// (e.g. implementing migration scripts or handling lazy migrations on reading old schema versions).
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

    fn max_attesters(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxAttesters)
            .unwrap_or(DEFAULT_MAX_ATTESTERS)
    }

    fn attester_count(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::AttesterCount)
            .unwrap_or(0)
    }

    fn record_status_change(env: &Env, attester: &Address, active: bool) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AttesterStatusChangeCount(attester.clone()))
            .unwrap_or(0);
        let sequence = count + 1;
        env.storage().persistent().set(
            &DataKey::AttesterStatusChange(attester.clone(), sequence),
            &AttesterStatusChange {
                timestamp: env.ledger().timestamp(),
                active,
            },
        );
        env.storage().persistent().set(
            &DataKey::AttesterStatusChangeCount(attester.clone()),
            &sequence,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod enumeration_spike;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod large_test;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
