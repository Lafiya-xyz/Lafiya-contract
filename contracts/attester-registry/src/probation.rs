//! Probation for newly enrolled attesters (issue #589).
//!
//! While probation is configured, every fresh enrollment starts a probation
//! window. During the window the attester is `Probation` (or `Flagged` once a
//! registrar or guardian flags it); consumers read the limits through
//! `get_probation_limits` / `is_payout_eligible`. State is derived lazily from
//! the ledger timestamp, so automatic promotion costs no transaction.
//! See `docs/specs/attester-probation.md`.

use soroban_sdk::{contractevent, contractimpl, contracttype, Address, Env, Symbol};

use crate::{
    AttesterRegistry, AttesterRegistryArgs, AttesterRegistryClient, DataKey, Error, Role,
    INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD,
};

/// Admin-configured probation policy applied to new enrollments.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbationConfig {
    /// Length of the probation window in seconds. `0` disables probation for
    /// future enrollments.
    pub duration: u64,
    /// Maximum attestations per `rate_window_ledgers` while on probation.
    /// `0` leaves the rate uncapped.
    pub rate_cap: u32,
    /// Window, in ledgers, over which `rate_cap` applies.
    pub rate_window_ledgers: u32,
    /// Whether a probationary attester may receive incentive-pool payouts.
    pub payout_eligible: bool,
}

/// Per-attester probation window, created at enrollment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbationRecord {
    pub started_at: u64,
    pub ends_at: u64,
    /// A flagged attester is not auto-promoted at `ends_at`; it needs an
    /// explicit `promote_attester`.
    pub flagged: bool,
}

/// Probation state of an attester at the current ledger time.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbationState {
    /// No probation applies (never enrolled under probation, expired
    /// unflagged, or promoted).
    Standard,
    /// Inside the probation window.
    Probation,
    /// Flagged: probation limits apply indefinitely until promotion.
    Flagged,
}

/// Limits applying to an attester that is not `Standard`, as read by the pool
/// and the attestation registry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbationLimits {
    pub state: ProbationState,
    pub ends_at: u64,
    pub rate_cap: u32,
    pub rate_window_ledgers: u32,
    pub payout_eligible: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationConfigured {
    pub duration: u64,
    pub rate_cap: u32,
    pub rate_window_ledgers: u32,
    pub payout_eligible: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationStarted {
    #[topic]
    pub attester: Address,
    pub ends_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationFlagged {
    #[topic]
    pub attester: Address,
    pub reason: Symbol,
}

/// Emitted when a probationary attester breaches a probation limit; the
/// watchdog consumes this alongside `ProbationStarted`.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationBreach {
    #[topic]
    pub attester: Address,
    pub kind: Symbol,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationPromoted {
    #[topic]
    pub attester: Address,
    /// `true` for `promote_attester`, `false` for expiry via `finalize_probation`.
    pub early: bool,
}

pub(crate) fn config(env: &Env) -> Option<ProbationConfig> {
    env.storage().instance().get(&DataKey::ProbationConfig)
}

fn record(env: &Env, attester: &Address) -> Option<ProbationRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::Probation(attester.clone()))
}

fn save(env: &Env, attester: &Address, rec: &ProbationRecord) {
    let key = DataKey::Probation(attester.clone());
    env.storage().persistent().set(&key, rec);
    env.storage()
        .persistent()
        .extend_ttl(&key, INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Start probation for a freshly enrolled attester, if probation is configured.
pub(crate) fn begin(env: &Env, attester: &Address) {
    let Some(cfg) = config(env) else { return };
    if cfg.duration == 0 {
        return;
    }
    let started_at = env.ledger().timestamp();
    let ends_at = started_at.saturating_add(cfg.duration);
    save(
        env,
        attester,
        &ProbationRecord {
            started_at,
            ends_at,
            flagged: false,
        },
    );
    ProbationStarted {
        attester: attester.clone(),
        ends_at,
    }
    .publish(env);
}

/// Drop probation state when an attester leaves the allowlist, so
/// re-enrolling under the same key starts a fresh window.
pub(crate) fn clear(env: &Env, attester: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Probation(attester.clone()));
}

/// Flag probation if still running; used when an attester is suspended.
pub(crate) fn flag_if_active(env: &Env, attester: &Address, reason: &Symbol) {
    if let Some(mut rec) = record(env, attester) {
        if !rec.flagged && env.ledger().timestamp() < rec.ends_at {
            rec.flagged = true;
            save(env, attester, &rec);
            ProbationFlagged {
                attester: attester.clone(),
                reason: reason.clone(),
            }
            .publish(env);
        }
    }
}

/// Carry probation state across a key rotation.
pub(crate) fn rotate(env: &Env, previous: &Address, new: &Address) {
    if let Some(rec) = record(env, previous) {
        clear(env, previous);
        save(env, new, &rec);
    }
}

fn state_of(env: &Env, attester: &Address) -> (ProbationState, Option<ProbationRecord>) {
    match record(env, attester) {
        None => (ProbationState::Standard, None),
        Some(rec) if rec.flagged => (ProbationState::Flagged, Some(rec)),
        Some(rec) if env.ledger().timestamp() >= rec.ends_at => (ProbationState::Standard, None),
        Some(rec) => (ProbationState::Probation, Some(rec)),
    }
}

pub(crate) fn require_registrar_or_guardian(env: &Env, caller: &Address) -> Result<(), Error> {
    let storage = env.storage().instance();
    let role = if storage.has(&DataKey::Role(Role::Registrar, caller.clone())) {
        Role::Registrar
    } else {
        Role::Guardian
    };
    AttesterRegistry::require_role(env, role, caller)
}

#[contractimpl]
impl AttesterRegistry {
    /// Set the probation policy applied to future enrollments. Requires the
    /// admin's authorization. Existing probation windows keep their end time
    /// but pick up the new caps.
    pub fn set_probation_config(
        env: Env,
        duration: u64,
        rate_cap: u32,
        rate_window_ledgers: u32,
        payout_eligible: bool,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if rate_cap > 0 && rate_window_ledgers == 0 {
            return Err(Error::InvalidProbationConfig);
        }
        env.storage().instance().set(
            &DataKey::ProbationConfig,
            &ProbationConfig {
                duration,
                rate_cap,
                rate_window_ledgers,
                payout_eligible,
            },
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        ProbationConfigured {
            duration,
            rate_cap,
            rate_window_ledgers,
            payout_eligible,
        }
        .publish(&env);
        Ok(())
    }

    /// The configured probation policy, if one has been set.
    pub fn get_probation_config(env: Env) -> Option<ProbationConfig> {
        config(&env)
    }

    /// Probation state of `attester` at the current ledger time.
    pub fn get_probation_state(env: Env, attester: Address) -> ProbationState {
        state_of(&env, &attester).0
    }

    /// Limits applying to `attester`, or `None` when it is `Standard`.
    pub fn get_probation_limits(env: Env, attester: Address) -> Option<ProbationLimits> {
        let (state, rec) = state_of(&env, &attester);
        let (rec, cfg) = (rec?, config(&env)?);
        Some(ProbationLimits {
            state,
            ends_at: rec.ends_at,
            rate_cap: cfg.rate_cap,
            rate_window_ledgers: cfg.rate_window_ledgers,
            payout_eligible: cfg.payout_eligible,
        })
    }

    /// Whether `attester` may receive incentive-pool payouts under probation
    /// rules. Always `true` for `Standard` attesters.
    pub fn is_payout_eligible(env: Env, attester: Address) -> bool {
        Self::get_probation_limits(env, attester).is_none_or(|limits| limits.payout_eligible)
    }

    /// Flag a probationary attester so it is not auto-promoted at expiry.
    /// Requires the registrar or guardian role.
    pub fn flag_probation(
        env: Env,
        caller: Address,
        attester: Address,
        reason: Symbol,
    ) -> Result<(), Error> {
        require_registrar_or_guardian(&env, &caller)?;
        let (state, rec) = state_of(&env, &attester);
        let mut rec = rec.ok_or(Error::ProbationNotFound)?;
        if state != ProbationState::Flagged {
            rec.flagged = true;
            save(&env, &attester, &rec);
            ProbationFlagged { attester, reason }.publish(&env);
        }
        Ok(())
    }

    /// Record a probation-limit breach (for example a rate or payout breach
    /// detected by the watchdog). Emits `ProbationBreach` and flags the
    /// attester. Requires the registrar or guardian role.
    pub fn report_probation_breach(
        env: Env,
        caller: Address,
        attester: Address,
        kind: Symbol,
    ) -> Result<(), Error> {
        require_registrar_or_guardian(&env, &caller)?;
        let (state, rec) = state_of(&env, &attester);
        let mut rec = rec.ok_or(Error::ProbationNotFound)?;
        if state != ProbationState::Flagged {
            rec.flagged = true;
            save(&env, &attester, &rec);
        }
        ProbationBreach { attester, kind }.publish(&env);
        Ok(())
    }

    /// End probation early and clear any flag. Requires a global registrar.
    pub fn promote_attester(env: Env, registrar: Address, attester: Address) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        if record(&env, &attester).is_none() {
            return Err(Error::ProbationNotFound);
        }
        clear(&env, &attester);
        ProbationPromoted {
            attester,
            early: true,
        }
        .publish(&env);
        Ok(())
    }

    /// Clear the record of an unflagged attester whose probation has expired
    /// and emit `ProbationPromoted`. Callable by anyone; purely housekeeping,
    /// as expiry already took effect.
    pub fn finalize_probation(env: Env, attester: Address) -> Result<(), Error> {
        match record(&env, &attester) {
            Some(rec) if !rec.flagged && env.ledger().timestamp() >= rec.ends_at => {
                clear(&env, &attester);
                ProbationPromoted {
                    attester,
                    early: false,
                }
                .publish(&env);
                Ok(())
            }
            _ => Err(Error::ProbationNotFound),
        }
    }
}
