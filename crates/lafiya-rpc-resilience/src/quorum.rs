//! Quorum reads for security-sensitive queries (issue #613).
//!
//! A single RPC provider can lie about attester status or attestation
//! existence, or serve stale state, and so produce a false "verified"
//! verdict. [`evaluate`] cross-checks the same read across independent
//! providers and only returns [`QuorumVerdict::Agreed`] when at least `k`
//! ledger-aligned providers return the identical value and no fresh provider
//! contradicts them. Anything else maps to a conservative verdict the caller
//! must treat as "not verified".
//!
//! # Trust assumptions
//! - At most `k - 1` of the configured providers are malicious or colluding;
//!   providers must be operated independently (different operators/infra).
//! - Freshness is judged against the highest ledger any provider reports, so
//!   a provider lying about its ledger *upward* can make honest ones look
//!   stale (a denial of verification, never a false positive).
//! - Any disagreement between fresh providers is treated as an attack or
//!   fault, even when `k` of them agree: availability is traded for safety.

use crate::RpcError;
use std::time::{Duration, Instant};

/// One provider's answer to a read, with the ledger it was read at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation<T> {
    pub provider: String,
    pub ledger: u32,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuorumPolicy {
    /// Minimum number of agreeing, fresh providers.
    pub k: usize,
    /// Providers more than this many ledgers behind the newest are stale.
    pub max_ledger_skew: u32,
    /// Stop querying once `k` agree. Lowers latency but gives up detecting a
    /// liar among the providers not yet asked.
    pub stop_at_k: bool,
}

impl Default for QuorumPolicy {
    fn default() -> Self {
        QuorumPolicy {
            k: 2,
            max_ledger_skew: 2,
            stop_at_k: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuorumVerdict<T> {
    Agreed {
        value: T,
        agreeing: usize,
    },
    /// Fresh providers returned different values. Treat as unverified.
    Disagreement,
    /// Too few fresh, successful answers to reach `k`. Treat as unverified.
    Insufficient,
}

/// Counters for export to the metrics pipeline.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QuorumMetrics {
    pub reads: u64,
    pub disagreements: u64,
    pub stale_excluded: u64,
    pub provider_failures: u64,
    pub insufficient: u64,
    pub last_latency: Duration,
}

/// Decide a verdict from already-collected observations.
pub fn evaluate<T: Clone + PartialEq>(
    results: Vec<Result<Observation<T>, RpcError>>,
    policy: QuorumPolicy,
    metrics: &mut QuorumMetrics,
) -> QuorumVerdict<T> {
    metrics.reads += 1;
    let mut ok = Vec::new();
    for r in results {
        match r {
            Ok(o) => ok.push(o),
            Err(_) => metrics.provider_failures += 1,
        }
    }
    let newest = ok.iter().map(|o| o.ledger).max().unwrap_or(0);
    let before = ok.len();
    ok.retain(|o| newest - o.ledger <= policy.max_ledger_skew);
    metrics.stale_excluded += (before - ok.len()) as u64;

    let Some(first) = ok.first() else {
        metrics.insufficient += 1;
        return QuorumVerdict::Insufficient;
    };
    if ok.iter().any(|o| o.value != first.value) {
        metrics.disagreements += 1;
        return QuorumVerdict::Disagreement;
    }
    if ok.len() >= policy.k.max(1) {
        QuorumVerdict::Agreed {
            value: first.value.clone(),
            agreeing: ok.len(),
        }
    } else {
        metrics.insufficient += 1;
        QuorumVerdict::Insufficient
    }
}

/// A provider read, as a closure so callers can wrap any client.
pub type QuorumSource<'a, T> = Box<dyn FnMut() -> Result<Observation<T>, RpcError> + 'a>;

/// Query `sources` in order (callers may parallelise their own closures),
/// measure the latency overhead into `metrics.last_latency`, and evaluate.
pub fn read_quorum<T: Clone + PartialEq>(
    sources: &mut [QuorumSource<'_, T>],
    policy: QuorumPolicy,
    metrics: &mut QuorumMetrics,
) -> QuorumVerdict<T> {
    let start = Instant::now();
    let mut results: Vec<Result<Observation<T>, RpcError>> = Vec::new();
    for s in sources.iter_mut() {
        results.push(s());
        if policy.stop_at_k && agreeing_prefix(&results) >= policy.k {
            break;
        }
    }
    let verdict = evaluate(results, policy, metrics);
    metrics.last_latency = start.elapsed();
    verdict
}

fn agreeing_prefix<T: PartialEq>(results: &[Result<Observation<T>, RpcError>]) -> usize {
    let mut oks = results.iter().filter_map(|r| r.as_ref().ok());
    let Some(first) = oks.next() else { return 0 };
    1 + oks.filter(|o| o.value == first.value).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(p: &str, ledger: u32, v: bool) -> Result<Observation<bool>, RpcError> {
        Ok(Observation {
            provider: p.into(),
            ledger,
            value: v,
        })
    }

    #[test]
    fn honest_providers_agree() {
        let mut m = QuorumMetrics::default();
        let v = evaluate(
            vec![obs("a", 100, true), obs("b", 101, true)],
            QuorumPolicy::default(),
            &mut m,
        );
        assert_eq!(
            v,
            QuorumVerdict::Agreed {
                value: true,
                agreeing: 2
            }
        );
    }

    #[test]
    fn lying_provider_is_never_trusted() {
        let mut m = QuorumMetrics::default();
        let v = evaluate(
            vec![
                obs("liar", 100, true),
                obs("honest", 100, false),
                obs("honest2", 100, false),
            ],
            QuorumPolicy::default(),
            &mut m,
        );
        assert_eq!(v, QuorumVerdict::Disagreement);
        assert_eq!(m.disagreements, 1);
    }

    #[test]
    fn stale_provider_is_excluded_and_cannot_form_quorum() {
        let mut m = QuorumMetrics::default();
        // The stale provider says "attester active"; the fresh one disagrees
        // with nothing but cannot reach k=2 alone.
        let v = evaluate(
            vec![obs("stale", 50, true), obs("fresh", 100, false)],
            QuorumPolicy::default(),
            &mut m,
        );
        assert_eq!(v, QuorumVerdict::Insufficient);
        assert_eq!(m.stale_excluded, 1);
    }

    #[test]
    fn one_liar_and_one_stale_provider_yield_safe_outcome() {
        let mut m = QuorumMetrics::default();
        let v = evaluate(
            vec![
                obs("liar", 100, true),
                obs("stale", 10, true),
                obs("honest", 100, false),
            ],
            QuorumPolicy::default(),
            &mut m,
        );
        assert_eq!(v, QuorumVerdict::Disagreement);
    }

    #[test]
    fn failures_count_and_reduce_quorum() {
        let mut m = QuorumMetrics::default();
        let v = evaluate(
            vec![obs("a", 100, true), Err(RpcError::Timeout)],
            QuorumPolicy::default(),
            &mut m,
        );
        assert_eq!(v, QuorumVerdict::Insufficient);
        assert_eq!(m.provider_failures, 1);
    }

    #[test]
    fn stop_at_k_skips_remaining_sources_and_records_latency() {
        let mut m = QuorumMetrics::default();
        let mut calls = 0;
        let policy = QuorumPolicy {
            stop_at_k: true,
            ..Default::default()
        };
        let mut sources: Vec<QuorumSource<'_, bool>> = vec![
            Box::new(|| obs("a", 100, true)),
            Box::new(|| obs("b", 100, true)),
            Box::new(|| {
                calls += 1;
                obs("c", 100, false)
            }),
        ];
        let v = read_quorum(&mut sources, policy, &mut m);
        drop(sources);
        assert_eq!(
            v,
            QuorumVerdict::Agreed {
                value: true,
                agreeing: 2
            }
        );
        assert_eq!(calls, 0);
    }
}
