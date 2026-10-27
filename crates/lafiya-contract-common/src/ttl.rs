//! TTL (time-to-live) management for contract storage.
//!
//! Soroban contracts must actively extend the TTL of instance and persistent
//! storage, or their data will be archived. These helpers provide the canonical
//! TTL policy for Lafiya contracts.

use soroban_sdk::Env;

/// Instance storage must be extended when it has less than this many ledger
/// seconds left. Typical value is ~6 days (518,400 seconds).
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// When extending instance storage TTL, bump it by this many ledger seconds.
/// Typical value is ~18 days (1,555,200 seconds).
pub const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;

/// Maximum horizon (in ledger seconds) that any single TTL extension may
/// reach. Extensions are clamped to this bound so that permissionless callers
/// cannot pin entries indefinitely. Typical value is ~1 year (31,536,000
/// seconds).
pub const MAX_TTL_HORIZON: u32 = 31_536_000;

/// Minimum horizon (in ledger seconds) an extension request must reach.
/// Requests below this are rejected rather than silently shortened.
pub const MIN_TTL_HORIZON: u32 = INSTANCE_BUMP_AMOUNT;

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
    // Approximate ledger close time in seconds.
    const SECONDS_PER_LEDGER: u64 = 5;
    // Approximate rent per ledger for a small persistent entry, in stroops.
    const RENT_PER_LEDGER_STROOPS: u64 = 1;
    let ledgers = (horizon as u64) / SECONDS_PER_LEDGER;
    ledgers.saturating_mul(RENT_PER_LEDGER_STROOPS)
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
}
