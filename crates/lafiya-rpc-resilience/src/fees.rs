//! Resource-fee margins, inclusion-fee percentile bidding, and fee-bump
//! decisions for Soroban submissions (see issue #408).
//!
//! Soroban fees have two independent parts:
//! - a **resource fee**, computed from simulation (instructions, ledger
//!   entry reads, and bytes read/written) -- taken verbatim today, so a
//!   small state change between simulation and inclusion (e.g. the
//!   attester-registry's entry count changing the read footprint) makes the
//!   transaction fail with `txSorobanInvalid` / a resource-limit error;
//! - an **inclusion fee**, the bid for block space during surge pricing --
//!   fixed today, so an admin transaction (including an emergency `pause`)
//!   can sit unconfirmed for minutes under congestion.
//!
//! This module is deliberately dependency-free (like the rest of this
//! crate, see `lib.rs`'s module doc): it takes already-parsed resource
//! usage and fee-stats numbers rather than an RPC client or a JSON
//! deserializer, so a caller (the CLI, or a future async RPC client) owns
//! parsing `simulateTransaction`/`getFeeStats` responses and this module
//! stays a pure, easily-tested calculator over the resulting numbers.

/// A resource-fee-relevant footprint, as reported by `simulateTransaction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceUsage {
    pub instructions: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

/// Network-wide per-transaction resource maximums (from the network's
/// current config; conservative defaults are provided for tests and for a
/// caller that hasn't fetched them yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkLimits {
    pub max_instructions: u64,
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
}

impl Default for NetworkLimits {
    /// Testnet-era Soroban maximums as of this writing. A real caller should
    /// prefer fetching current limits from `getNetwork`/`getLedgerEntries`
    /// (`ConfigSettingContractComputeV0`/`...LedgerCostV0`) over relying on
    /// this default, which exists mainly so `apply_margin` has a sane cap in
    /// tests and for a first run before that fetch is wired up.
    fn default() -> Self {
        NetworkLimits {
            max_instructions: 100_000_000,
            max_read_bytes: 200_000,
            max_write_bytes: 132_096,
        }
    }
}

/// Add a `margin_percent` safety margin to each simulated resource,
/// capped at `limits` so an inflated margin never itself becomes the reason
/// the transaction is rejected. `margin_percent` is whole percent (e.g. `15`
/// for +15%).
pub fn apply_margin(
    usage: ResourceUsage,
    margin_percent: u32,
    limits: NetworkLimits,
) -> ResourceUsage {
    let bump = |value: u64, cap: u64| -> u64 {
        let scaled = value.saturating_mul(100 + u64::from(margin_percent)) / 100;
        scaled.min(cap)
    };
    ResourceUsage {
        instructions: bump(usage.instructions, limits.max_instructions),
        read_bytes: bump(usage.read_bytes, limits.max_read_bytes),
        write_bytes: bump(usage.write_bytes, limits.max_write_bytes),
    }
}

/// A parsed `getFeeStats` response's `sorobanInclusionFee` percentile
/// ladder (stroops). Mirrors the RPC's field set narrowly to what this
/// module needs; a caller parses the full JSON response and constructs one
/// of these from the fields it cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeStats {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
}

/// How urgently a submission needs to land. Emergency admin commands
/// (`pause`, `emergency-stop`, `revoke`) default to `High`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Normal,
    High,
}

/// Bid `stats`' p50 for `Normal` priority, p90 for `High` -- the percentiles
/// named in the issue's proposal.
pub fn choose_inclusion_fee(stats: FeeStats, priority: Priority) -> u64 {
    match priority {
        Priority::Normal => stats.p50,
        Priority::High => stats.p90,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeError {
    /// The computed fee exceeds the operator-set `--max-fee` ceiling.
    ExceedsBudget { computed: u64, max_fee: u64 },
}

impl std::fmt::Display for FeeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeeError::ExceedsBudget { computed, max_fee } => write!(
                f,
                "computed fee {computed} stroops exceeds --max-fee budget of {max_fee} stroops"
            ),
        }
    }
}

/// Enforce an operator-set `--max-fee` ceiling. `None` means no ceiling was
/// configured (uncapped). Applied identically to the initial fee and to
/// every fee-bump escalation, so `--max-fee` bounds the whole retry
/// sequence, not just the first attempt.
pub fn enforce_budget(computed_fee: u64, max_fee: Option<u64>) -> Result<u64, FeeError> {
    match max_fee {
        Some(max) if computed_fee > max => Err(FeeError::ExceedsBudget {
            computed: computed_fee,
            max_fee: max,
        }),
        _ => Ok(computed_fee),
    }
}

/// A decision on whether to wrap a still-pending transaction in a
/// [`FeeBumpTransaction`](https://developers.stellar.org/docs/encyclopedia/fee-bump-transactions)
/// with a higher inclusion fee. The inner transaction's hash is unchanged by
/// a fee bump, so ADR-0011's poll-by-hash recovery logic
/// (`crates/lafiya-rpc-resilience`'s `FailoverClient`) applies unmodified to
/// the wrapped submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBumpDecision {
    /// The new inclusion fee to bid, already checked against `--max-fee`.
    pub new_inclusion_fee: u64,
}

/// Decide whether a transaction pending for `ledgers_pending` (still
/// unconfirmed after that many ledgers closed) should be fee-bumped, and by
/// how much.
///
/// Returns `Ok(None)` if `ledgers_pending < threshold` (not stuck yet).
/// Returns `Err` if the bumped fee would exceed `max_fee`; the caller should
/// keep polling at the current fee rather than escalate past budget.
pub fn decide_fee_bump(
    ledgers_pending: u32,
    threshold_ledgers: u32,
    current_inclusion_fee: u64,
    stats: FeeStats,
    max_fee: Option<u64>,
) -> Result<Option<FeeBumpDecision>, FeeError> {
    if ledgers_pending < threshold_ledgers {
        return Ok(None);
    }
    // Escalate to at least the high-priority percentile, and strictly above
    // whatever was already bid -- a fee bump that doesn't raise the bid
    // wouldn't change the transaction's position in surge pricing.
    let candidate = choose_inclusion_fee(stats, Priority::High).max(current_inclusion_fee + 1);
    let bounded = enforce_budget(candidate, max_fee)?;
    Ok(Some(FeeBumpDecision {
        new_inclusion_fee: bounded,
    }))
}

/// Append a human-readable line recording a resource-fee margin decision to
/// `log`, so a failure-injection or live run's log can be diffed against the
/// runbook the same way `crate::RecoveryLog` already supports for RPC
/// failover (see `crate::RecoveryLog`'s doc comment).
pub fn log_resource_margin(
    log: &mut crate::RecoveryLog,
    simulated: ResourceUsage,
    submitted: ResourceUsage,
) {
    log.record(format!(
        "resource fee: simulated instructions={} read_bytes={} write_bytes={}; submitted (margined) instructions={} read_bytes={} write_bytes={}",
        simulated.instructions,
        simulated.read_bytes,
        simulated.write_bytes,
        submitted.instructions,
        submitted.read_bytes,
        submitted.write_bytes,
    ));
}

/// Append a human-readable line recording a fee-bump decision to `log`.
pub fn log_fee_bump(log: &mut crate::RecoveryLog, previous_fee: u64, decision: FeeBumpDecision) {
    log.record(format!(
        "fee bump: {previous_fee} -> {} stroops inclusion fee",
        decision.new_inclusion_fee
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> ResourceUsage {
        ResourceUsage {
            instructions: 10_000_000,
            read_bytes: 20_000,
            write_bytes: 5_000,
        }
    }

    #[test]
    fn margin_adds_the_configured_percent() {
        let margined = apply_margin(usage(), 15, NetworkLimits::default());
        assert_eq!(margined.instructions, 11_500_000);
        assert_eq!(margined.read_bytes, 23_000);
        assert_eq!(margined.write_bytes, 5_750);
    }

    #[test]
    fn margin_is_capped_at_network_maximums() {
        let limits = NetworkLimits {
            max_instructions: 10_500_000, // just above simulated usage
            max_read_bytes: 200_000,
            max_write_bytes: 132_096,
        };
        let margined = apply_margin(usage(), 15, limits);
        // 10_000_000 * 1.15 = 11_500_000, which exceeds the 10_500_000 cap.
        assert_eq!(margined.instructions, 10_500_000);
    }

    #[test]
    fn margin_never_panics_near_u64_max() {
        let usage = ResourceUsage {
            instructions: u64::MAX,
            read_bytes: u64::MAX,
            write_bytes: u64::MAX,
        };
        let limits = NetworkLimits::default();
        let margined = apply_margin(usage, 15, limits);
        assert_eq!(margined.instructions, limits.max_instructions);
    }

    // Fixture `getFeeStats` responses, as a caller would construct them
    // after parsing the RPC's JSON (this crate is dependency-free -- see
    // the module doc -- so it takes the already-parsed numbers).
    fn quiet_network_fee_stats() -> FeeStats {
        FeeStats { p50: 100, p90: 100, p99: 100 }
    }

    fn congested_network_fee_stats() -> FeeStats {
        FeeStats { p50: 100, p90: 50_000, p99: 500_000 }
    }

    #[test]
    fn normal_priority_bids_p50() {
        assert_eq!(
            choose_inclusion_fee(congested_network_fee_stats(), Priority::Normal),
            100
        );
    }

    #[test]
    fn high_priority_bids_p90() {
        assert_eq!(
            choose_inclusion_fee(congested_network_fee_stats(), Priority::High),
            50_000
        );
    }

    #[test]
    fn budget_guard_rejects_fee_over_max() {
        let err = enforce_budget(1000, Some(500)).unwrap_err();
        assert_eq!(
            err,
            FeeError::ExceedsBudget {
                computed: 1000,
                max_fee: 500
            }
        );
    }

    #[test]
    fn budget_guard_allows_fee_at_or_under_max() {
        assert_eq!(enforce_budget(500, Some(500)).unwrap(), 500);
        assert_eq!(enforce_budget(100, None).unwrap(), 100);
    }

    #[test]
    fn fee_bump_not_triggered_before_threshold() {
        let decision =
            decide_fee_bump(2, 5, 100, congested_network_fee_stats(), None).unwrap();
        assert_eq!(decision, None);
    }

    #[test]
    fn fee_bump_triggers_at_threshold_and_bids_p90() {
        let decision =
            decide_fee_bump(5, 5, 100, congested_network_fee_stats(), None)
                .unwrap()
                .unwrap();
        assert_eq!(decision.new_inclusion_fee, 50_000);
    }

    #[test]
    fn fee_bump_always_strictly_increases_even_on_a_quiet_network() {
        // p90 == p50 == current fee here; the bump must still move the bid
        // up, or it wouldn't change the transaction's queue position.
        let decision =
            decide_fee_bump(5, 5, 100, quiet_network_fee_stats(), None)
                .unwrap()
                .unwrap();
        assert_eq!(decision.new_inclusion_fee, 101);
    }

    #[test]
    fn log_resource_margin_records_both_values() {
        let mut log = crate::RecoveryLog::new();
        log_resource_margin(&mut log, usage(), apply_margin(usage(), 15, NetworkLimits::default()));
        assert_eq!(log.lines().len(), 1);
        assert!(log.lines()[0].contains("simulated instructions=10000000"));
        assert!(log.lines()[0].contains("submitted (margined) instructions=11500000"));
    }

    #[test]
    fn log_fee_bump_records_old_and_new_fee() {
        let mut log = crate::RecoveryLog::new();
        log_fee_bump(&mut log, 100, FeeBumpDecision { new_inclusion_fee: 50_000 });
        assert_eq!(log.lines(), ["fee bump: 100 -> 50000 stroops inclusion fee"]);
    }

    #[test]
    fn fee_bump_respects_max_fee_budget() {
        let err = decide_fee_bump(5, 5, 100, congested_network_fee_stats(), Some(1000))
            .unwrap_err();
        assert_eq!(
            err,
            FeeError::ExceedsBudget {
                computed: 50_000,
                max_fee: 1000
            }
        );
    }
}
