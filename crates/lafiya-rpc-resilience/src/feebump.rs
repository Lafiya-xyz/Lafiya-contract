//! Fee-bump submission and sponsor-account monitoring (issue #612).
//!
//! [`crate::fees::decide_fee_bump`] decides *whether* a stuck transaction
//! should be re-bid; this module drives it. A fee bump wraps the original
//! (inner) transaction in a `FeeBumpTransaction` signed by the sponsor, so the
//! inner hash -- and therefore ADR-0011's poll-by-hash recovery -- is
//! unchanged, and the inner sequence number can only be consumed once. A bump
//! can therefore never produce a second on-chain effect.

use crate::fees::{decide_fee_bump, log_fee_bump, FeeError, FeeStats};
use crate::{RecoveryLog, RpcProvider, SignedTx, SubmitOutcome, TxState};

/// Limits on fee escalation for one intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBumpPolicy {
    /// Ledgers a transaction may stay pending before it is bumped.
    pub timeout_ledgers: u32,
    /// Hard ceiling on the inclusion fee (stroops); `None` is uncapped.
    pub max_fee: Option<u64>,
    /// Maximum number of bumps, bounding runaway escalation.
    pub max_bumps: u32,
    /// Maximum number of status polls before giving up.
    pub max_polls: u32,
}

impl Default for FeeBumpPolicy {
    fn default() -> Self {
        FeeBumpPolicy {
            timeout_ledgers: 5,
            max_fee: Some(10_000_000),
            max_bumps: 3,
            max_polls: 60,
        }
    }
}

/// Wraps an inner transaction in a sponsor-signed fee-bump envelope. The
/// returned [`SignedTx`] must keep the inner transaction's hash.
pub trait FeeBumpBuilder {
    fn build(&mut self, inner: &SignedTx, inclusion_fee: u64) -> SignedTx;
}

impl<F: FnMut(&SignedTx, u64) -> SignedTx> FeeBumpBuilder for F {
    fn build(&mut self, inner: &SignedTx, inclusion_fee: u64) -> SignedTx {
        self(inner, inclusion_fee)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeeBumpResult {
    Accepted {
        ledger: u32,
        bumps: u32,
        final_fee: u64,
    },
    Rejected {
        reason: String,
    },
    /// The inner transaction's `maxLedger` passed; it can never land.
    Expired,
    /// Polling ended without a verdict. The inner hash is still the handle to
    /// resume with; nothing was duplicated.
    Unresolved {
        bumps: u32,
        final_fee: u64,
    },
}

/// Submit `inner`, bumping its fee via `builder` when it stays pending.
/// `sponsor_balance` (stroops) prevents bidding more than the sponsor holds.
#[allow(clippy::too_many_arguments)]
pub fn submit_with_fee_bump(
    provider: &mut dyn RpcProvider,
    inner: &SignedTx,
    initial_fee: u64,
    stats: FeeStats,
    policy: FeeBumpPolicy,
    sponsor_balance: Option<u64>,
    builder: &mut dyn FeeBumpBuilder,
    log: &mut RecoveryLog,
) -> FeeBumpResult {
    let mut fee = initial_fee;
    let mut bumps = 0u32;
    let mut capped = false;
    match provider.submit(inner) {
        SubmitOutcome::Ack(TxState::Rejected { reason }) => {
            return FeeBumpResult::Rejected { reason }
        }
        SubmitOutcome::Ack(TxState::Expired { .. }) => return FeeBumpResult::Expired,
        // Ambiguous/definite failures are left to the polling loop below: the
        // hash tells us whether the network ever saw it.
        _ => {}
    }
    let mut pending_since = provider.latest_ledger().unwrap_or(0);

    for _ in 0..policy.max_polls {
        match provider.get_transaction(&inner.hash) {
            Ok(TxState::Accepted { ledger }) => {
                return FeeBumpResult::Accepted {
                    ledger,
                    bumps,
                    final_fee: fee,
                }
            }
            Ok(TxState::Rejected { reason }) => return FeeBumpResult::Rejected { reason },
            Ok(TxState::Expired { .. }) => return FeeBumpResult::Expired,
            _ => {}
        }
        if capped || bumps >= policy.max_bumps {
            continue;
        }
        let Ok(latest) = provider.latest_ledger() else {
            continue;
        };
        let pending = latest.saturating_sub(pending_since);
        match decide_fee_bump(pending, policy.timeout_ledgers, fee, stats, policy.max_fee) {
            Ok(None) => {}
            Err(FeeError::ExceedsBudget { computed, max_fee }) => {
                log.record(format!(
                    "fee bump halted: {computed} stroops exceeds max fee {max_fee}"
                ));
                capped = true;
            }
            Ok(Some(d)) if sponsor_balance.is_some_and(|b| d.new_inclusion_fee > b) => {
                log.record(format!(
                    "fee bump halted: sponsor balance cannot cover {} stroops",
                    d.new_inclusion_fee
                ));
                capped = true;
            }
            Ok(Some(d)) => {
                log_fee_bump(log, fee, d);
                let bumped = builder.build(inner, d.new_inclusion_fee);
                // Same inner hash: a lost/ambiguous response is resolved by
                // the next poll, never by building a new transaction.
                let _ = provider.submit(&bumped);
                fee = d.new_inclusion_fee;
                bumps += 1;
                pending_since = latest;
            }
        }
    }
    FeeBumpResult::Unresolved {
        bumps,
        final_fee: fee,
    }
}

/// Alert level for a sponsor account's native balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SponsorAlert {
    Ok,
    /// Below the warning threshold: top up soon.
    Low {
        balance: u64,
    },
    /// Below what is needed to sponsor a worst-case transaction: stop
    /// accepting sponsored work until topped up.
    Critical {
        balance: u64,
    },
}

/// Balance thresholds (stroops) for a sponsor account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SponsorMonitor {
    pub low_water: u64,
    /// Worst-case cost of one sponsored transaction (the policy `max_fee`).
    pub critical: u64,
}

impl SponsorMonitor {
    pub fn assess(&self, balance: u64) -> SponsorAlert {
        if balance < self.critical {
            SponsorAlert::Critical { balance }
        } else if balance < self.low_water {
            SponsorAlert::Low { balance }
        } else {
            SponsorAlert::Ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::ScriptedProvider;

    const STATS: FeeStats = FeeStats {
        p50: 100,
        p90: 500,
        p99: 2000,
    };

    fn builder() -> impl FnMut(&SignedTx, u64) -> SignedTx {
        |inner: &SignedTx, fee| SignedTx::new(inner.hash.clone(), format!("bump:{fee}"))
    }

    fn stuck_then_accepted() -> ScriptedProvider {
        ScriptedProvider::new("p")
            .then_submit(SubmitOutcome::Ack(TxState::Pending))
            .then_submit(SubmitOutcome::Ack(TxState::Pending))
            .then_latest_ledger(100)
            .then_latest_ledger(106)
            .then_poll(Ok(TxState::Pending))
            .then_poll(Ok(TxState::Pending))
            .then_poll(Ok(TxState::Accepted { ledger: 108 }))
    }

    #[test]
    fn bumps_after_timeout_without_duplicate_effect() {
        let mut p = stuck_then_accepted();
        let inner = SignedTx::new("abc", "inner");
        let mut log = RecoveryLog::new();
        let r = submit_with_fee_bump(
            &mut p,
            &inner,
            100,
            STATS,
            FeeBumpPolicy::default(),
            None,
            &mut builder(),
            &mut log,
        );
        assert_eq!(
            r,
            FeeBumpResult::Accepted {
                ledger: 108,
                bumps: 1,
                final_fee: 500
            }
        );
        // one original + one bump submission, both for the same inner hash
        assert_eq!(p.submit_calls, 2);
        assert!(log
            .lines()
            .iter()
            .any(|l| l.contains("fee bump: 100 -> 500")));
    }

    #[test]
    fn escalation_stops_at_max_fee() {
        let mut p = stuck_then_accepted();
        let inner = SignedTx::new("abc", "inner");
        let policy = FeeBumpPolicy {
            max_fee: Some(300),
            ..Default::default()
        };
        let mut log = RecoveryLog::new();
        let r = submit_with_fee_bump(
            &mut p,
            &inner,
            100,
            STATS,
            policy,
            None,
            &mut builder(),
            &mut log,
        );
        assert_eq!(
            r,
            FeeBumpResult::Accepted {
                ledger: 108,
                bumps: 0,
                final_fee: 100
            }
        );
        assert_eq!(p.submit_calls, 1);
        assert!(log.lines().iter().any(|l| l.contains("exceeds max fee")));
    }

    #[test]
    fn unaffordable_bump_is_refused() {
        let mut p = stuck_then_accepted();
        let inner = SignedTx::new("abc", "inner");
        let mut log = RecoveryLog::new();
        let r = submit_with_fee_bump(
            &mut p,
            &inner,
            100,
            STATS,
            FeeBumpPolicy::default(),
            Some(200),
            &mut builder(),
            &mut log,
        );
        assert!(matches!(r, FeeBumpResult::Accepted { bumps: 0, .. }));
        assert_eq!(p.submit_calls, 1);
    }

    #[test]
    fn sponsor_alert_levels() {
        let m = SponsorMonitor {
            low_water: 1_000,
            critical: 100,
        };
        assert_eq!(m.assess(5_000), SponsorAlert::Ok);
        assert_eq!(m.assess(500), SponsorAlert::Low { balance: 500 });
        assert_eq!(m.assess(50), SponsorAlert::Critical { balance: 50 });
    }
}
