//! TTL (time-to-live) management for contract storage.
//!
//! Soroban contracts must actively extend the TTL of instance and persistent
//! storage, or their data will be archived. These helpers provide the canonical
//! TTL policy for Lafiya contracts.

use soroban_sdk::Env;

use crate::time::{Ledgers, Seconds};

/// Declared ledger close-time assumption used when converting between
/// [`Seconds`] and [`Ledgers`] for TTL policy. See
/// `docs/architecture/storage-versioning.md` for the rationale.
pub const TTL_CLOSE_TIME: Seconds = Seconds::new(5);

/// Safety factor applied to the close-time assumption when converting a
/// validity window into a ledger count. A value of `2` means we assume the
/// ledger can close up to 50% faster than declared, so a validity window
/// always maps to *more* ledgers than the nominal conversion.
pub const TTL_SAFETY_FACTOR: u32 = 2;

/// Instance storage must be extended when it has less than this many ledger
/// seconds left. Typical value is ~6 days (518,400 seconds).
pub const INSTANCE_LIFETIME_THRESHOLD_SECONDS: Seconds = Seconds::new(518_400);

/// When extending instance storage TTL, bump it by this many ledger seconds.
/// Typical value is ~18 days (1,555,200 seconds).
pub const INSTANCE_BUMP_AMOUNT_SECONDS: Seconds = Seconds::new(1_555_200);

/// Maximum horizon (in ledger seconds) that any single TTL extension may
/// reach. Extensions are clamped to this bound so that permissionless callers
/// cannot pin entries indefinitely. Typical value is ~1 year (31,536,000
/// seconds).
pub const MAX_TTL_HORIZON_SECONDS: Seconds = Seconds::new(31_536_000);

/// Instance storage must be extended when it has less than this many ledger
/// seconds left, as a raw `u32` for Soroban storage APIs.
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_LIFETIME_THRESHOLD_SECONDS.as_u32();

/// When extending instance storage TTL, bump it by this many ledger seconds,
/// as a raw `u32` for Soroban storage APIs.
pub const INSTANCE_BUMP_AMOUNT: u32 = INSTANCE_BUMP_AMOUNT_SECONDS.as_u32();

/// Maximum horizon (in ledger seconds) that any single TTL extension may
/// reach, as a raw `u32` for Soroban storage APIs.
pub const MAX_TTL_HORIZON: u32 = MAX_TTL_HORIZON_SECONDS.as_u32();

/// Minimum horizon (in ledger seconds) an extension request must reach.
/// Requests below this are rejected rather than silently shortened.
pub const MIN_TTL_HORIZON: u32 = INSTANCE_BUMP_AMOUNT;

/// Convert a validity window in [`Seconds`] into the number of [`Ledgers`]
/// that must be retained so the entry outlives its validity even if the
/// ledger closes up to 50% faster than the declared assumption.
pub fn ledgers_covering(validity: Seconds) -> Ledgers {
    validity.to_ledgers_with_safety(TTL_CLOSE_TIME, TTL_SAFETY_FACTOR)
}

/// Extend the instance storage TTL if it falls below the threshold.
/// Call this at the end of any state-changing operation.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Clamp a requested extension horizon into the allowed `[MIN_TTL_HORIZON,
/// MAX_TTL_HORIZON]` range.
///
/// Returns `None` when the request is below `MIN_TTL_HORIZON` (callers should
/// reject such requests so TTL can never be shortened), otherwise returns the
/// requested horizon capped at `MAX_TTL_HORIZON`.
pub fn clamp_extend_to(extend_to: u32) -> Option<u32> {
    if extend_to < MIN_TTL_HORIZON {
        return None;
    }
    Some(if extend_to > MAX_TTL_HORIZON {
        MAX_TTL_HORIZON
    } else {
        extend_to
    })
}

/// Compute the effective extension horizon for a persistent entry.
///
/// The result is the requested horizon clamped to `MAX_TTL_HORIZON`, and never
/// less than the entry's current remaining TTL, so an extension can only ever
/// keep or grow the lifetime of an entry.
pub fn bounded_extend_to(current_remaining: u32, extend_to: u32) -> u32 {
    let capped = if extend_to > MAX_TTL_HORIZON {
        MAX_TTL_HORIZON
    } else {
        extend_to
    };
    if capped < current_remaining {
        current_remaining
    } else {
        capped
    }
}

/// Extend a persistent storage entry's TTL to a bounded horizon.
///
/// `extend_to` is clamped via [`bounded_extend_to`] so the entry can never be
/// shortened and never exceeds `MAX_TTL_HORIZON`. Returns the horizon applied.
pub fn extend_persistent_ttl<K>(env: &Env, key: &K, extend_to: u32) -> u32
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    let current = env.storage().persistent().get_ttl(key);
    let target = bounded_extend_to(current, extend_to);
    env.storage().persistent().extend_ttl(key, target, target);
    target
}

/// Estimate the rent (in stroops) required to keep a single persistent entry
/// alive for `horizon` ledger seconds.
///
/// This is a deterministic approximation used for budgeting and for the CLI
/// `restore` helper. It is intentionally conservative: it assumes a fixed
/// per-entry footprint and the network base rent rate.
pub fn rent_per_entry_stroops(horizon: u32) -> u64 {
    // Approximate rent per ledger for a small persistent entry, in stroops.
    const RENT_PER_LEDGER_STROOPS: u64 = 1;
    let ledgers = Seconds::new(horizon).to_ledgers(TTL_CLOSE_TIME);
    (ledgers.as_u64()).saturating_mul(RENT_PER_LEDGER_STROOPS)
}

/// Estimate the total rent (in stroops) for `count` persistent entries kept
/// alive for `horizon` ledger seconds. Used to document cost at scale.
pub fn rent_for_entries_stroops(count: u32, horizon: u32) -> u64 {
    rent_per_entry_stroops(horizon).saturating_mul(count as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn ttl_constants_are_reasonable() {
        // Threshold should be less than bump to avoid thrashing
        assert!(
            INSTANCE_LIFETIME_THRESHOLD < INSTANCE_BUMP_AMOUNT,
            "TTL threshold ({}) must be less than bump amount ({})",
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT
        );

        // Both should be positive
        assert!(
            INSTANCE_LIFETIME_THRESHOLD > 0,
            "TTL threshold must be positive"
        );
        assert!(INSTANCE_BUMP_AMOUNT > 0, "TTL bump amount must be positive");

        // Sanity check: both should be measured in ledger seconds (~5s per ledger)
        // 518,400 seconds ≈ 6 days; 1,555,200 seconds ≈ 18 days
        assert!(
            INSTANCE_LIFETIME_THRESHOLD > 86_400,
            "TTL threshold ({}) should represent at least 1 day",
            INSTANCE_LIFETIME_THRESHOLD
        );
        assert!(
            INSTANCE_BUMP_AMOUNT > 604_800,
            "TTL bump amount ({}) should represent at least 1 week",
            INSTANCE_BUMP_AMOUNT
        );
    }

    #[test]
    fn max_horizon_exceeds_min_horizon() {
        assert!(
            MIN_TTL_HORIZON < MAX_TTL_HORIZON,
            "min horizon ({}) must be less than max horizon ({})",
            MIN_TTL_HORIZON,
            MAX_TTL_HORIZON
        );
    }

    #[test]
    fn clamp_extend_to_rejects_shortening() {
        assert_eq!(clamp_extend_to(0), None);
        assert_eq!(clamp_extend_to(MIN_TTL_HORIZON - 1), None);
        assert_eq!(clamp_extend_to(MIN_TTL_HORIZON), Some(MIN_TTL_HORIZON));
    }

    #[test]
    fn clamp_extend_to_caps_at_max() {
        assert_eq!(clamp_extend_to(MAX_TTL_HORIZON), Some(MAX_TTL_HORIZON));
        assert_eq!(
            clamp_extend_to(MAX_TTL_HORIZON + 1),
            Some(MAX_TTL_HORIZON)
        );
        assert_eq!(clamp_extend_to(u32::MAX), Some(MAX_TTL_HORIZON));
    }

    #[test]
    fn bounded_extend_to_never_shortens() {
        assert_eq!(bounded_extend_to(1_000, 0), 1_000);
        assert_eq!(bounded_extend_to(1_000, 500), 1_000);
        assert_eq!(bounded_extend_to(1_000, 2_000), 2_000);
        assert_eq!(bounded_extend_to(1_000, u32::MAX), MAX_TTL_HORIZON);
    }

    #[test]
    fn rent_scales_linearly() {
        let one = rent_per_entry_stroops(MAX_TTL_HORIZON);
        assert!(one > 0);
        assert_eq!(rent_for_entries_stroops(10, MAX_TTL_HORIZON), one * 10);
        assert_eq!(rent_for_entries_stroops(0, MAX_TTL_HORIZON), 0);
    }

    #[test]
    fn typed_constants_match_raw_accessors() {
        assert_eq!(INSTANCE_LIFETIME_THRESHOLD, 518_400);
        assert_eq!(INSTANCE_BUMP_AMOUNT, 1_555_200);
        assert_eq!(MAX_TTL_HORIZON, 31_536_000);
        assert_eq!(INSTANCE_LIFETIME_THRESHOLD_SECONDS.as_u32(), 518_400);
        assert_eq!(INSTANCE_BUMP_AMOUNT_SECONDS.as_u32(), 1_555_200);
        assert_eq!(MAX_TTL_HORIZON_SECONDS.as_u32(), 31_536_000);
    }

    #[test]
    fn instance_ttl_covers_validity_at_faster_close_time() {
        // The instance bump must retain the entry for at least its declared
        // validity window even if ledgers close 50% faster than assumed.
        let required = ledgers_covering(INSTANCE_BUMP_AMOUNT_SECONDS);
        let provided = Seconds::new(INSTANCE_BUMP_AMOUNT).to_ledgers_with_safety(
            TTL_CLOSE_TIME,
            TTL_SAFETY_FACTOR,
        );
        assert!(
            provided >= required,
            "instance bump ({}) must cover validity ({}) at a faster close time",
            provided.as_u64(),
            required.as_u64()
        );
    }

    #[test]
    fn instance_threshold_covers_validity_at_faster_close_time() {
        let required = ledgers_covering(INSTANCE_LIFETIME_THRESHOLD_SECONDS);
        let provided = Seconds::new(INSTANCE_LIFETIME_THRESHOLD).to_ledgers_with_safety(
            TTL_CLOSE_TIME,
            TTL_SAFETY_FACTOR,
        );
        assert!(
            provided >= required,
            "instance threshold ({}) must cover validity ({}) at a faster close time",
            provided.as_u64(),
            required.as_u64()
        );
    }

    #[test]
    fn max_horizon_covers_validity_at_faster_close_time() {
        let required = ledgers_covering(MAX_TTL_HORIZON_SECONDS);
        let provided = Seconds::new(MAX_TTL_HORIZON).to_ledgers_with_safety(
            TTL_CLOSE_TIME,
            TTL_SAFETY_FACTOR,
        );
        assert!(
            provided >= required,
            "max horizon ({}) must cover validity ({}) at a faster close time",
            provided.as_u64(),
            required.as_u64()
        );
    }
}
