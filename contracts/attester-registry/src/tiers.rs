//! Capability tiers (issue #588).
//!
//! Each attester carries a `Tier`. A capability matrix, stored as data and
//! changed only through a timelocked propose/apply flow, maps an attestation
//! class to the minimum tier allowed to attest it. Classes absent from the
//! matrix require only the base tier, and attesters without an assigned tier
//! default to the base tier, so existing deployments behave as before.
//! See `docs/specs/attester-tiers.md`.

use soroban_sdk::{contractevent, contractimpl, contracttype, Address, Env, Map, Symbol};

use crate::{
    AttesterRegistry, AttesterRegistryArgs, AttesterRegistryClient, DataKey, Error, Role,
    StoredAttesterInfo, INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD,
};

/// Delay between proposing and applying a capability matrix change.
pub const TIER_MATRIX_TIMELOCK_SECONDS: u64 = 2 * 24 * 60 * 60;

/// Upper bound on matrix entries, keeping the instance entry small.
pub const MAX_TIER_CLASSES: u32 = 32;

/// Attester qualification level. Ordered: a higher tier satisfies any
/// requirement for a lower one.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Tier {
    /// Community health worker; the default for unassigned attesters.
    Chw = 1,
    Nurse = 2,
    Physician = 3,
}

/// A proposed capability matrix waiting out its timelock.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingTierMatrix {
    pub matrix: Map<Symbol, Tier>,
    pub executable_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterTierSet {
    #[topic]
    pub attester: Address,
    pub tier: Tier,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct TierMatrixProposed {
    pub executable_at: u64,
    pub entries: u32,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct TierMatrixApplied {
    pub entries: u32,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct TierMatrixCancelled {
    pub entries: u32,
}

fn matrix(env: &Env) -> Map<Symbol, Tier> {
    env.storage()
        .instance()
        .get(&DataKey::TierMatrix)
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn tier_of(env: &Env, attester: &Address) -> Tier {
    env.storage()
        .persistent()
        .get(&DataKey::AttesterTier(attester.clone()))
        .unwrap_or(Tier::Chw)
}

pub(crate) fn clear(env: &Env, attester: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::AttesterTier(attester.clone()));
}

/// Carry the tier across a key rotation.
pub(crate) fn rotate(env: &Env, previous: &Address, new: &Address) {
    let key = DataKey::AttesterTier(previous.clone());
    if let Some(tier) = env.storage().persistent().get::<_, Tier>(&key) {
        env.storage().persistent().remove(&key);
        store(env, new, tier);
    }
}

fn store(env: &Env, attester: &Address, tier: Tier) {
    let key = DataKey::AttesterTier(attester.clone());
    env.storage().persistent().set(&key, &tier);
    env.storage()
        .persistent()
        .extend_ttl(&key, INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

#[contractimpl]
impl AttesterRegistry {
    /// Assign `attester` a capability tier. Requires a global registrar.
    pub fn set_attester_tier(
        env: Env,
        registrar: Address,
        attester: Address,
        tier: Tier,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        env.storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester.clone()))
            .filter(|info| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        store(&env, &attester, tier);
        AttesterTierSet { attester, tier }.publish(&env);
        Ok(())
    }

    /// The attester's tier; `Tier::Chw` when none was assigned.
    pub fn get_attester_tier(env: Env, attester: Address) -> Tier {
        tier_of(&env, &attester)
    }

    /// Propose a replacement capability matrix (class -> minimum tier). It
    /// can be applied after `TIER_MATRIX_TIMELOCK_SECONDS`. Requires the
    /// admin; replaces any earlier pending proposal.
    pub fn propose_tier_matrix(env: Env, new_matrix: Map<Symbol, Tier>) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if new_matrix.len() > MAX_TIER_CLASSES {
            return Err(Error::TierMatrixTooLarge);
        }
        let executable_at = env
            .ledger()
            .timestamp()
            .saturating_add(TIER_MATRIX_TIMELOCK_SECONDS);
        let entries = new_matrix.len();
        env.storage().instance().set(
            &DataKey::PendingTierMatrix,
            &PendingTierMatrix {
                matrix: new_matrix,
                executable_at,
            },
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        TierMatrixProposed {
            executable_at,
            entries,
        }
        .publish(&env);
        Ok(())
    }

    /// Apply the pending capability matrix once its timelock has elapsed.
    /// Requires the admin.
    pub fn apply_tier_matrix(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let pending: PendingTierMatrix = env
            .storage()
            .instance()
            .get(&DataKey::PendingTierMatrix)
            .ok_or(Error::NoPendingTierMatrix)?;
        if env.ledger().timestamp() < pending.executable_at {
            return Err(Error::TierMatrixTimelocked);
        }
        let entries = pending.matrix.len();
        env.storage()
            .instance()
            .set(&DataKey::TierMatrix, &pending.matrix);
        env.storage().instance().remove(&DataKey::PendingTierMatrix);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        TierMatrixApplied { entries }.publish(&env);
        Ok(())
    }

    /// Discard the pending capability matrix. Requires the admin.
    pub fn cancel_tier_matrix(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let pending: PendingTierMatrix = env
            .storage()
            .instance()
            .get(&DataKey::PendingTierMatrix)
            .ok_or(Error::NoPendingTierMatrix)?;
        env.storage().instance().remove(&DataKey::PendingTierMatrix);
        TierMatrixCancelled {
            entries: pending.matrix.len(),
        }
        .publish(&env);
        Ok(())
    }

    /// The active capability matrix (class -> minimum tier).
    pub fn get_tier_matrix(env: Env) -> Map<Symbol, Tier> {
        matrix(&env)
    }

    /// The proposed capability matrix and when it becomes applicable.
    pub fn get_pending_tier_matrix(env: Env) -> Option<PendingTierMatrix> {
        env.storage().instance().get(&DataKey::PendingTierMatrix)
    }

    /// Minimum tier required to attest `class`; `Tier::Chw` if unlisted.
    pub fn get_required_tier(env: Env, class: Symbol) -> Tier {
        matrix(&env).get(class).unwrap_or(Tier::Chw)
    }

    /// Whether `attester` is currently allowlisted and its tier satisfies the
    /// matrix requirement for `class`.
    pub fn is_class_allowed(env: Env, attester: Address, class: Symbol) -> bool {
        Self::is_attester(env.clone(), attester.clone())
            && tier_of(&env, &attester) >= Self::get_required_tier(env, class)
    }
}
