//! Probation for newly enrolled attesters (issues #589, #678).
//!
//! While probation is configured, every fresh enrollment starts a probation
//! window. During the window the attester is `Probation` (or `Flagged` once a
//! registrar or guardian flags it); consumers read the limits through
//! `get_probation_limits` / `is_payout_eligible`. State is derived lazily from
//! the ledger timestamp, so automatic promotion costs no transaction.
//!
//! Exit from probation is objective and counter-based: `try_promote` is
//! permissionless and checks the versioned [`ProbationPolicy`] criteria against
//! per-attester counters (attestations, upheld disputes, revocations). Admins
//! can still `extend_probation` (or `promote_attester` for an override).
//!
//! # Call graph (acyclic)
//!
//! ```text
//! attestation-registry --(record_attestation / record_dispute / record_revocation)-->
//!     attester-registry
//! ```
//!
//! The attestation registry pushes counters into this contract through narrow
//! hooks; this contract never calls back into the attestation registry or the
//! incentive pool. The incentive pool only reads `is_payout_eligible`. There are
//! no circular cross-contract dependencies.
//!
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

/// Versioned, objective promotion criteria (issue #678).
///
/// A probationary attester may be promoted permissionlessly via `try_promote`
/// once it satisfies every criterion below. Bump `version` whenever the
/// semantics change so off-chain consumers can interpret counters correctly.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbationPolicy {
    /// Policy schema version.
    pub version: u32,
    /// Minimum number of attestations recorded while on probation.
    pub min_attestations: u32,
    /// Minimum tenure, in seconds, since probation started.
    pub min_tenure_secs: u64,
    /// Maximum upheld disputes tolerated (inclusive).
    pub max_upheld_disputes: u32,
    /// Maximum revocations tolerated (inclusive).
    pub max_revocations: u32,
}

/// Current `ProbationPolicy` schema version.
pub const PROBATION_POLICY_VERSION: u32 = 1;

impl ProbationPolicy {
    /// Default v1 criteria.
    pub fn v1() -> Self {
        Self {
            version: PROBATION_POLICY_VERSION,
            min_attestations: 10,
            min_tenure_secs: 7 * 24 * 60 * 60,
            max_upheld_disputes: 0,
            max_revocations: 0,
        }
    }
}

/// Per-attester counters feeding the promotion criteria. Updated only through
/// the narrow hooks below, pushed by the attestation registry.
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProbationCounters {
    pub attestations: u32,
    pub upheld_disputes: u32,
    pub revocations: u32,
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

/// Emitted when `try_promote` succeeds and the attester leaves probation.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationCompleted {
    #[topic]
    pub attester: Address,
    pub policy_version: u32,
    pub attestations: u32,
    pub tenure_secs: u64,
}

/// Emitted when the promotion policy is updated.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationPolicyUpdated {
    pub version: u32,
    pub min_attestations: u32,
    pub min_tenure_secs: u64,
    pub max_upheld_disputes: u32,
    pub max_revocations: u32,
}

/// Emitted when an admin extends a probation window.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ProbationExtended {
    #[topic]
    pub attester: Address,
    pub ends_at: u64,
}

pub(crate) fn config(env: &Env) -> Option<ProbationConfig> {
    env.storage().instance().get(&DataKey::ProbationConfig)
}

fn record(env: &Env, attester: &Address) -> Option<ProbationRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::Probation(attester.clone()))
}

/// The active promotion policy, falling back to the v1 defaults.
pub(crate) fn policy(env: &Env) -> ProbationPolicy {
    env.storage()
        .instance()
        .get(&DataKey::ProbationPolicy)
        .unwrap_or_else(ProbationPolicy::v1)
}

fn counters(env: &Env, attester: &Address) -> ProbationCounters {
    env.storage()
        .persistent()
        .get(&DataKey::ProbationCounters(attester.clone()))
        .unwrap_or_default()
}

fn save_counters(env: &Env, attester: &Address, c: &ProbationCounters) {
    let key = DataKey::ProbationCounters(attester.clone());
    env.storage().persistent().set(&key, c);
    env.storage()
        .persistent()
        .extend_ttl(&key, INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Whether `attester` currently satisfies every promotion criterion.
fn meets_criteria(
    env: &Env,
    rec: &ProbationRecord,
    c: &ProbationCounters,
    p: &ProbationPolicy,
) -> bool {
    let tenure = env.ledger().timestamp().saturating_sub(rec.started_at);
    c.attestations >= p.min_attestations
        && tenure >= p.min_tenure_secs
        && c.upheld_disputes <= p.max_upheld_disputes
        && c.revocations <= p.max_revocations
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
    env.storage()
        .persistent()
        .remove(&DataKey::ProbationCounters(attester.clone()));
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
        let c = counters(env, previous);
        clear(env, previous);
        save(env, new, &rec);
        save_counters(env, new, &c);
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

    /// Set the versioned promotion criteria. Requires the admin's authorization.
    pub fn set_probation_policy(
        env: Env,
        min_attestations: u32,
        min_tenure_secs: u64,
        max_upheld_disputes: u32,
        max_revocations: u32,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let p = ProbationPolicy {
            version: PROBATION_POLICY_VERSION,
            min_attestations,
            min_tenure_secs,
            max_upheld_disputes,
            max_revocations,
        };
        env.storage()
            .instance()
            .set(&DataKey::ProbationPolicy, &p);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        ProbationPolicyUpdated {
            version: p.version,
            min_attestations,
            min_tenure_secs,
            max_upheld_disputes,
            max_revocations,
        }
        .publish(&env);
        Ok(())
    }

    /// The active promotion policy (v1 defaults if never set).
    pub fn get_probation_policy(env: Env) -> ProbationPolicy {
        policy(&env)
    }

    /// Counters feeding the promotion criteria for `attester`.
    pub fn get_probation_counters(env: Env, attester: Address) -> ProbationCounters {
        counters(&env, &attester)
    }

    /// Hook: record an attestation for a probationary attester. Pushed by the
    /// attestation registry; a no-op for attesters not on probation.
    pub fn record_probation_attestation(
        env: Env,
        caller: Address,
        attester: Address,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &caller)?;
        if record(&env, &attester).is_none() {
            return Ok(());
        }
        let mut c = counters(&env, &attester);
        c.attestations = c.attestations.saturating_add(1);
        save_counters(&env, &attester, &c);
        Ok(())
    }

    /// Hook: record an upheld dispute against a probationary attester.
    pub fn record_probation_dispute(
        env: Env,
        caller: Address,
        attester: Address,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &caller)?;
        if record(&env, &attester).is_none() {
            return Ok(());
        }
        let mut c = counters(&env, &attester);
        c.upheld_disputes = c.upheld_disputes.saturating_add(1);
        save_counters(&env, &attester, &c);
        Ok(())
    }

    /// Hook: record a revocation for a probationary attester.
    pub fn record_probation_revocation(
        env: Env,
        caller: Address,
        attester: Address,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &caller)?;
        if record(&env, &attester).is_none() {
            return Ok(());
        }
        let mut c = counters(&env, &attester);
        c.revocations = c.revocations.saturating_add(1);
        save_counters(&env, &attester, &c);
        Ok(())
    }

    /// Permissionlessly promote `attester` once it satisfies the versioned
    /// promotion criteria. Refuses with `ProbationCriteriaNotMet` otherwise.
    pub fn try_promote(env: Env, attester: Address) -> Result<(), Error> {
        Self::require_not_paused(&env)?;
        let rec = record(&env, &attester).ok_or(Error::ProbationNotFound)?;
        let c = counters(&env, &attester);
        let p = policy(&env);
        if !meets_criteria(&env, &rec, &c, &p) {
            return Err(Error::ProbationCriteriaNotMet);
        }
        let tenure_secs = env.ledger().timestamp().saturating_sub(rec.started_at);
        clear(&env, &attester);
        ProbationCompleted {
            attester,
            policy_version: p.version,
            attestations: c.attestations,
            tenure_secs,
        }
        .publish(&env);
        Ok(())
    }

    /// Extend a probation window by `extra_secs`. Requires a global registrar.
    pub fn extend_probation(
        env: Env,
        registrar: Address,
        attester: Address,
        extra_secs: u64,
    ) -> Result<(), Error> {
        Self::require_role(&env, Role::Registrar, &registrar)?;
        Self::require_not_paused(&env)?;
        let mut rec = record(&env, &attester).ok_or(Error::ProbationNotFound)?;
        rec.ends_at = rec.ends_at.saturating_add(extra_secs);
        save(&env, &attester, &rec);
        ProbationExtended {
            attester,
            ends_at: rec.ends_at,
        }
        .publish(&env);
        Ok(())
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

    /// Admin override: end probation early and clear any flag. Requires a
    /// global registrar. Prefer the permissionless `try_promote`; this remains
    /// for exceptional cases only.
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
