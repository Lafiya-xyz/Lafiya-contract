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

/// Extend the instance storage TTL if it falls below the threshold.
/// Call this at the end of any state-changing operation.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(INSTANCE_LIFETIME_THRESHOLD > 0, "TTL threshold must be positive");
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
}
