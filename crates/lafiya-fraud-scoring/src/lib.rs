//! Per-attester fraud/anomaly risk scoring over indexed attestation events
//! (issue #415). See `docs/spikes/0415-attester-fraud-scoring.md` for the
//! design writeup (features, thresholds, false-positive handling, and the
//! privacy review) and `docs/runbooks/attester-fraud-response.md` for the
//! flag -> review -> suspend operational playbook this feeds.
//!
//! **Privacy:** every input here is either an on-chain commitment
//! (`record_hash`, already just a hash -- see ADR-0001), a coarse region
//! code, or metadata the indexer already has (attester, ledger, timestamp,
//! revocation status). No patient data, and no data finer-grained than
//! "region", ever enters this module. See the design doc's privacy review
//! for why each field here is safe to compute over.
//!
//! This crate is dependency-free, like `lafiya-rpc-resilience`: it scores
//! already-indexed events rather than querying an indexer itself, so it can
//! be embedded in `lafiya-indexer` (or any other analytics job) without
//! pulling in that service's own dependencies.

use std::collections::{HashMap, HashSet};

/// One indexed attestation event. Constructed by the caller from indexer
/// storage (and, for `region`, an off-chain join against `lafiya-web` --
/// see the design doc).
#[derive(Debug, Clone)]
pub struct AttestationEvent {
    pub attester: String,
    pub record_hash: [u8; 32],
    /// Coarse region the *record* is associated with (off-chain join), used
    /// to detect region mismatch against the attester's own region.
    pub record_region: String,
    pub ledger: u32,
    pub timestamp_secs: u64,
    pub revoked: bool,
}

/// An attester's own registered region (from the allowlist / off-chain
/// licensing data), used as the comparison point for region mismatch.
#[derive(Debug, Clone)]
pub struct AttesterProfile {
    pub attester: String,
    pub region: String,
}

/// Thresholds the scoring job flags against. Defaults here are the
/// **illustrative** starting points from the issue's proposal (e.g. "200
/// verifications in an hour"), not calibrated production values -- the
/// design doc's "Thresholds" section covers how these should be tuned
/// against real regional data before this runs against production traffic.
#[derive(Debug, Clone)]
pub struct ScoringConfig {
    /// An hourly count above this is flagged as a physically implausible
    /// velocity, regardless of any regional baseline.
    pub max_plausible_hourly_attestations: u32,
    /// Two events are in the same "burst" if their ledgers are at most this
    /// many ledgers apart.
    pub burst_max_ledger_gap: u32,
    /// A burst run at least this long is flagged (scripted submission
    /// rather than field work).
    pub burst_min_run_length: u32,
    /// A record hash attested by the same attester more than this many
    /// times is flagged as re-attestation churn.
    pub churn_max_repeats: u32,
    /// A record-region mismatch rate above this fraction is flagged.
    pub region_mismatch_fraction_threshold: f64,
    /// An attester's revocation rate flagged if it exceeds the population's
    /// mean revocation rate multiplied by this factor ("far more often than
    /// peers", computed directly from the scored population rather than a
    /// fixed absolute rate).
    pub revocation_ratio_peer_multiplier: f64,
    /// Two attesters sharing at least this many record hashes are flagged
    /// as a possible collusion pair.
    pub collusion_shared_record_threshold: u32,
    pub high_severity_score: f64,
    pub medium_severity_score: f64,
    /// Attesters whose events are ignored entirely (e.g. the canary attester,
    /// see docs/runbooks/canary-attestations.md).
    pub excluded_attesters: Vec<String>,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        ScoringConfig {
            max_plausible_hourly_attestations: 50,
            burst_max_ledger_gap: 2,
            burst_min_run_length: 20,
            churn_max_repeats: 2,
            region_mismatch_fraction_threshold: 0.2,
            revocation_ratio_peer_multiplier: 3.0,
            collusion_shared_record_threshold: 5,
            high_severity_score: 3.0,
            medium_severity_score: 1.0,
            excluded_attesters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
}

/// The individual heuristic signals that can contribute to a score. Each
/// contributes a fixed weight (see [`SignalBreakdown::weight`]) when it
/// fires, so a reviewer can see exactly which signals drove a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Signal {
    Velocity,
    BurstTiming,
    ReAttestationChurn,
    RegionMismatch,
    RevocationRatio,
}

impl Signal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Signal::Velocity => "velocity",
            Signal::BurstTiming => "burst_timing",
            Signal::ReAttestationChurn => "re_attestation_churn",
            Signal::RegionMismatch => "region_mismatch",
            Signal::RevocationRatio => "revocation_ratio",
        }
    }
}

/// Structured, per-signal explainability record (issue #530). Every signal
/// that fires contributes one of these to a [`RiskReport`], carrying the
/// weight it added, the observed value that tripped it, and the baseline /
/// threshold it was compared against. This is what an appeal or a suspension
/// reason code cites.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalBreakdown {
    pub signal: Signal,
    pub weight: f64,
    /// Human-readable observed value (e.g. "80 attestations in one hour").
    pub observed: String,
    /// Human-readable baseline / threshold the observed value exceeded.
    pub baseline: String,
}

/// Per-attester scoring output. `reasons` is the explainability requirement
/// from the issue: every point added to `score` has a matching human-
/// readable reason, so a reviewer never has to reverse-engineer why an
/// attester was flagged. `breakdown` is the structured form of the same
/// information (issue #530).
#[derive(Debug, Clone)]
pub struct RiskReport {
    pub attester: String,
    pub score: f64,
    pub reasons: Vec<String>,
    pub breakdown: Vec<SignalBreakdown>,
    pub severity: Severity,
}

fn severity_for(score: f64, config: &ScoringConfig) -> Severity {
    if score >= config.high_severity_score {
        Severity::High
    } else if score >= config.medium_severity_score {
        Severity::Medium
    } else {
        Severity::Low
    }
}

/// Longest run of `ledgers` (sorted ascending) where each consecutive pair
/// is within `max_gap` of each other.
fn longest_burst_run(mut ledgers: Vec<u32>, max_gap: u32) -> u32 {
    if ledgers.is_empty() {
        return 0;
    }
    ledgers.sort_unstable();
    let mut best = 1u32;
    let mut current = 1u32;
    for window in ledgers.windows(2) {
        if window[1].saturating_sub(window[0]) <= max_gap {
            current += 1;
            best = best.max(current);
        } else {
            current = 1;
        }
    }
    best
}

/// Score every attester that appears in `events` against `profiles` and
/// `config`. Attesters with no flags are still returned, with `score: 0.0`
/// and `severity: Low` -- callers filtering for alerts should filter on
/// `severity`, not on presence in the returned `Vec`.
pub fn score_attesters(
    events: &[AttestationEvent],
    profiles: &[AttesterProfile],
    config: &ScoringConfig,
) -> Vec<RiskReport> {
    score_attesters_weighted(events, profiles, config, &ModelSpec::default_model())
}

/// Same as [`score_attesters`], but each signal's contribution is the weight
/// from `model` rather than a fixed `1.0`.
pub fn score_attesters_weighted(
    events: &[AttestationEvent],
    profiles: &[AttesterProfile],
    config: &ScoringConfig,
    model: &ModelSpec,
) -> Vec<RiskReport> {
    let profile_by_attester: HashMap<&str, &str> = profiles
        .iter()
        .map(|p| (p.attester.as_str(), p.region.as_str()))
        .collect();

    let mut by_attester: HashMap<&str, Vec<&AttestationEvent>> = HashMap::new();
    for event in events {
        if config.excluded_attesters.contains(&event.attester) {
            continue;
        }
        by_attester.entry(&event.attester).or_default().push(event);
    }

    // Population mean revocation rate, for the peer-relative revocation check.
    let population_mean_revocation_rate = {
        let rates: Vec<f64> = by_attester
            .values()
            .map(|evs| evs.iter().filter(|e| e.revoked).count() as f64 / evs.len() as f64)
            .collect();
        if rates.is_empty() {
            0.0
        } else {
            rates.iter().sum::<f64>() / rates.len() as f64
        }
    };

    let mut reports = Vec::with_capacity(by_attester.len());
    for (attester, evs) in &by_attester {
        let mut score = 0.0;
        let mut reasons = Vec::new();
        let mut breakdown = Vec::new();
        let w_velocity = model.weight(Signal::Velocity);
        let w_burst = model.weight(Signal::BurstTiming);
        let w_churn = model.weight(Signal::ReAttestationChurn);
        let w_region = model.weight(Signal::RegionMismatch);
        let w_revocation = model.weight(Signal::RevocationRatio);

        // Velocity: busiest 1-hour bucket.
        let mut hourly_counts: HashMap<u64, u32> = HashMap::new();
        for e in evs {
            *hourly_counts.entry(e.timestamp_secs / 3600).or_insert(0) += 1;
        }
        if let Some(&max_hourly) = hourly_counts.values().max() {
            if max_hourly > config.max_plausible_hourly_attestations {
                score += w_velocity;
                reasons.push(format!(
                    "velocity: {max_hourly} attestations in one hour (implausibility threshold {})",
                    config.max_plausible_hourly_attestations
                ));
                breakdown.push(SignalBreakdown {
                    signal: Signal::Velocity,
                    weight: 1.0,
                    observed: format!("{max_hourly} attestations in one hour"),
                    baseline: format!(
                        "max plausible {} per hour",
                        config.max_plausible_hourly_attestations
                    ),
                });
            }
        }

        // Burst timing: longest run of closely-spaced ledgers.
        let ledgers: Vec<u32> = evs.iter().map(|e| e.ledger).collect();
        let run = longest_burst_run(ledgers, config.burst_max_ledger_gap);
        if run >= config.burst_min_run_length {
            score += 1.0;
            reasons.push(format!(
                "burst timing: {run} attestations within {} ledgers of each other (threshold {})",
                config.burst_max_ledger_gap, config.burst_min_run_length
            ));
            breakdown.push(SignalBreakdown {
                signal: Signal::BurstTiming,
                weight: 1.0,
                observed: format!(
                    "{run} attestations within {} ledgers",
                    config.burst_max_ledger_gap
                ),
                baseline: format!("min run length {}", config.burst_min_run_length),
            });
        }

        // Re-attestation churn: same record hash attested repeatedly.
        let mut per_hash_counts: HashMap<[u8; 32], u32> = HashMap::new();
        for e in evs {
            *per_hash_counts.entry(e.record_hash).or_insert(0) += 1;
        }
        let max_repeats = per_hash_counts.values().copied().max().unwrap_or(0);
        if max_repeats > config.churn_max_repeats {
            score += 1.0;
            reasons.push(format!(
                "re-attestation churn: one record hash attested {max_repeats} times (threshold {})",
                config.churn_max_repeats
            ));
            breakdown.push(SignalBreakdown {
                signal: Signal::ReAttestationChurn,
                weight: 1.0,
                observed: format!("one record hash attested {max_repeats} times"),
                baseline: format!("max repeats {}", config.churn_max_repeats),
            });
        }

        // Region mismatch.
        if let Some(&own_region) = profile_by_attester.get(attester) {
            let mismatches = evs.iter().filter(|e| e.record_region != own_region).count();
            let fraction = mismatches as f64 / evs.len() as f64;
            if fraction > config.region_mismatch_fraction_threshold {
                score += 1.0;
                reasons.push(format!(
                    "region mismatch: {:.0}% of attestations outside the attester's registered region {own_region} (threshold {:.0}%)",
                    fraction * 100.0,
                    config.region_mismatch_fraction_threshold * 100.0
                ));
                breakdown.push(SignalBreakdown {
                    signal: Signal::RegionMismatch,
                    weight: 1.0,
                    observed: format!("{:.0}% out-of-region", fraction * 100.0),
                    baseline: format!(
                        "max {:.0}% out-of-region",
                        config.region_mismatch_fraction_threshold * 100.0
                    ),
                });
            }
        }

        // Revocation ratio, relative to the scored population's mean.
        let revoked = evs.iter().filter(|e| e.revoked).count();
        let revocation_rate = revoked as f64 / evs.len() as f64;
        let peer_threshold =
            population_mean_revocation_rate * config.revocation_ratio_peer_multiplier;
        if population_mean_revocation_rate > 0.0 && revocation_rate > peer_threshold {
            score += 1.0;
            reasons.push(format!(
                "revocation ratio: {:.0}% of this attester's attestations were later revoked, vs. a {:.0}% population mean ({}x)",
                revocation_rate * 100.0,
                population_mean_revocation_rate * 100.0,
                config.revocation_ratio_peer_multiplier
            ));
            breakdown.push(SignalBreakdown {
                signal: Signal::RevocationRatio,
                weight: 1.0,
                observed: format!("{:.0}% revoked", revocation_rate * 100.0),
                baseline: format!(
                    "{:.0}% population mean x{}",
                    population_mean_revocation_rate * 100.0,
                    config.revocation_ratio_peer_multiplier
                ),
            });
        }

        reports.push(RiskReport {
            attester: attester.to_string(),
            score,
            severity: severity_for(score, config),
            reasons,
            breakdown,
        });
    }

    reports.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap()
            .then(a.attester.cmp(&b.attester))
    });
    reports
}

/// A pair of attesters whose attested record sets overlap unusually often --
/// the "collusion graph" cluster-detection deliverable, reduced to pairwise
/// overlap counts (a full graph-clustering pass is a documented follow-up in
/// the design doc; pairwise overlap already surfaces the clearest cases and
/// is far simpler to review and explain).
#[derive(Debug, Clone)]
pub struct CollusionPair {
    pub attester_a: String,
    pub attester_b: String,
    pub shared_record_count: u32,
}

/// Find attester pairs that have both attested the same record hash at
/// least `config.collusion_shared_record_threshold` times.
pub fn find_collusion_pairs(
    events: &[AttestationEvent],
    config: &ScoringConfig,
) -> Vec<CollusionPair> {
    let mut attesters_by_hash: HashMap<[u8; 32], HashSet<&str>> = HashMap::new();
    for e in events {
        attesters_by_hash
            .entry(e.record_hash)
            .or_default()
            .insert(&e.attester);
    }

    let mut shared_counts: HashMap<(String, String), u32> = HashMap::new();
    for attesters in attesters_by_hash.values() {
        if attesters.len() < 2 {
            continue;
        }
        let mut sorted: Vec<&&str> = attesters.iter().collect();
        sorted.sort();
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                let key = (sorted[i].to_string(), sorted[j].to_string());
                *shared_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    let mut pairs: Vec<CollusionPair> = shared_counts
        .into_iter()
        .filter(|(_, count)| *count >= config.collusion_shared_record_threshold)
        .map(|((a, b), count)| CollusionPair {
            attester_a: a,
            attester_b: b,
            shared_record_count: count,
        })
        .collect();
    pairs.sort_by(|a, b| {
        b.shared_record_count
            .cmp(&a.shared_record_count)
            .then(a.attester_a.cmp(&b.attester_a))
    });
    pairs
}

/// Versioned, serializable model specification (issue #751). A `ModelSpec`
/// pins the model version, the feature set, the per-signal weights and the
/// thresholds, so every score can be attributed to an exact model. It
/// serializes to TOML and hashes deterministically (see
/// [`ModelSpec::to_toml`] / [`ModelSpec::model_hash`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    /// Monotonic model version, e.g. `"1"`, `"2"`.
    pub version: String,
    /// Feature names this model scores over, in canonical order.
    pub features: Vec<String>,
    /// Per-signal weights, in canonical [`Signal`] order.
    pub weights: Vec<(Signal, f64)>,
    /// The thresholds this model flags against.
    pub config: ScoringConfig,
}

impl ModelSpec {
    /// The default model: the illustrative thresholds from [`ScoringConfig`]
    /// with every signal weighted `1.0`.
    pub fn default_model() -> Self {
        ModelSpec {
            version: "1".to_string(),
            features: ALL_SIGNALS.iter().map(|s| s.as_str().to_string()).collect(),
            weights: ALL_SIGNALS.iter().map(|s| (*s, 1.0)).collect(),
            config: ScoringConfig::default(),
        }
    }

    /// Weight for `signal`, defaulting to `1.0` if the model omits it.
    pub fn weight(&self, signal: Signal) -> f64 {
        self.weights
            .iter()
            .find(|(s, _)| *s == signal)
            .map(|(_, w)| *w)
            .unwrap_or(1.0)
    }

    /// Canonical TOML serialization. Field order is fixed and floats are
    /// rendered with `{:?}` so the output is byte-for-byte reproducible.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("version = {:?}\n", self.version));
        out.push_str("features = [");
        for (i, f) in self.features.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{f:?}"));
        }
        out.push_str("]\n\n[weights]\n");
        for (signal, weight) in &self.weights {
            out.push_str(&format!("{} = {weight:?}\n", signal.as_str()));
        }
        out.push_str("\n[thresholds]\n");
        let c = &self.config;
        out.push_str(&format!(
            "max_plausible_hourly_attestations = {}\n",
            c.max_plausible_hourly_attestations
        ));
        out.push_str(&format!("burst_max_ledger_gap = {}\n", c.burst_max_ledger_gap));
        out.push_str(&format!(
            "burst_min_run_length = {}\n",
            c.burst_min_run_length
        ));
        out.push_str(&format!("churn_max_repeats = {}\n", c.churn_max_repeats));
        out.push_str(&format!(
            "region_mismatch_fraction_threshold = {:?}\n",
            c.region_mismatch_fraction_threshold
        ));
        out.push_str(&format!(
            "revocation_ratio_peer_multiplier = {:?}\n",
            c.revocation_ratio_peer_multiplier
        ));
        out.push_str(&format!(
            "collusion_shared_record_threshold = {}\n",
            c.collusion_shared_record_threshold
        ));
        out.push_str(&format!("high_severity_score = {:?}\n", c.high_severity_score));
        out.push_str(&format!(
            "medium_severity_score = {:?}\n",
            c.medium_severity_score
        ));
        out.push_str("excluded_attesters = [");
        for (i, a) in c.excluded_attesters.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{a:?}"));
        }
        out.push_str("]\n");
        out
    }

    /// Deterministic hash of the canonical TOML form (FNV-1a 64, hex).
    pub fn model_hash(&self) -> String {
        fnv1a64_hex(self.to_toml().as_bytes())
    }
}

/// All signals in canonical order.
pub const ALL_SIGNALS: [Signal; 5] = [
    Signal::Velocity,
    Signal::BurstTiming,
    Signal::ReAttestationChurn,
    Signal::RegionMismatch,
    Signal::RevocationRatio,
];

/// FNV-1a 64-bit hash rendered as lowercase hex. Dependency-free and stable
/// across platforms, which is what replay reproducibility needs.
pub fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Digest of the exact inputs a score was computed from, so a score can be
/// replayed and verified bit for bit.
pub fn input_digest(events: &[AttestationEvent], profiles: &[AttesterProfile]) -> String {
    let mut buf = String::new();
    for e in events {
        buf.push_str(&format!(
            "e|{}|{}|{}|{}|{}|{}\n",
            e.attester,
            hex32(&e.record_hash),
            e.record_region,
            e.ledger,
            e.timestamp_secs,
            e.revoked
        ));
    }
    for p in profiles {
        buf.push_str(&format!("p|{}|{}\n", p.attester, p.region));
    }
    fnv1a64_hex(buf.as_bytes())
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// A score report plus the provenance needed to attribute and replay it.
#[derive(Debug, Clone)]
pub struct ScoredReport {
    pub report: RiskReport,
    pub model_hash: String,
    pub input_digest: String,
}

/// Score with an explicit [`ModelSpec`], attaching `model_hash` and
/// `input_digest` to every report.
pub fn score_with_model(
    events: &[AttestationEvent],
    profiles: &[AttesterProfile],
    model: &ModelSpec,
) -> Vec<ScoredReport> {
    let model_hash = model.model_hash();
    let digest = input_digest(events, profiles);
    score_attesters_weighted(events, profiles, &model.config, model)
        .into_iter()
        .map(|report| ScoredReport {
            report,
            model_hash: model_hash.clone(),
            input_digest: digest.clone(),
        })
        .collect()
}

/// Replay: recompute scores from the same inputs and model, and verify the
/// result matches the historical reports bit for bit.
pub fn replay(
    events: &[AttestationEvent],
    profiles: &[AttesterProfile],
    model: &ModelSpec,
    historical: &[ScoredReport],
) -> Result<Vec<ScoredReport>, String> {
    let digest = input_digest(events, profiles);
    if let Some(h) = historical.first() {
        if h.input_digest != digest {
            return Err(format!(
                "input digest mismatch: historical {} vs replay {}",
                h.input_digest, digest
            ));
        }
    }
    let replayed = score_with_model(events, profiles, model);
    if replayed.len() != historical.len() {
        return Err(format!(
            "report count mismatch: historical {} vs replay {}",
            historical.len(),
            replayed.len()
        ));
    }
    for (a, b) in historical.iter().zip(replayed.iter()) {
        if a.report.attester != b.report.attester
            || a.report.score.to_bits() != b.report.score.to_bits()
            || a.report.severity != b.report.severity
            || a.model_hash != b.model_hash
            || a.input_digest != b.input_digest
        {
            return Err(format!(
                "score mismatch for attester {}: historical {} vs replay {}",
                a.report.attester, a.report.score, b.report.score
            ));
        }
    }
    Ok(replayed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_for(n: u64) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(&n.to_be_bytes());
        h
    }

    /// One honest attester's timeline: one attestation every 20 minutes
    /// (3/hour), each a distinct record, correct region, never revoked.
    fn honest_attester_events(
        attester: &str,
        region: &str,
        start_hash: u64,
        count: u32,
    ) -> Vec<AttestationEvent> {
        (0..count)
            .map(|i| AttestationEvent {
                attester: attester.to_string(),
                record_hash: hash_for(start_hash + i as u64),
                record_region: region.to_string(),
                ledger: i * 100,                   // widely spaced
                timestamp_secs: (i as u64) * 1200, // every 20 minutes
                revoked: false,
            })
            .collect()
    }

    /// A fraudulent attester's timeline: a scripted burst of consecutive
    /// ledgers, all within one hour, many re-attesting the same handful of
    /// hashes, mostly out-of-region, with a high revocation rate.
    fn fraudulent_attester_events(attester: &str, own_region: &str) -> Vec<AttestationEvent> {
        (0..80u32)
            .map(|i| AttestationEvent {
                attester: attester.to_string(),
                record_hash: hash_for((i % 5) as u64 + 1000), // churns 5 hashes
                record_region: "out-of-region".to_string(),
                ledger: i,                       // consecutive ledgers: one big burst
                timestamp_secs: (i as u64) * 10, // 80 events inside ~13 minutes
                revoked: i % 2 == 0,             // 50% revoked
            })
            .map(|mut e| {
                if e.ledger % 7 == 0 {
                    e.record_region = own_region.to_string(); // a few in-region, to be realistic
                }
                e
            })
            .collect()
    }

    #[test]
    fn honest_population_is_not_flagged() {
        let mut events = Vec::new();
        let mut profiles = Vec::new();
        for i in 0..10 {
            let attester = format!("honest-{i}");
            events.extend(honest_attester_events(
                &attester,
                "region-a",
                i as u64 * 1000,
                30,
            ));
            profiles.push(AttesterProfile {
                attester,
                region: "region-a".to_string(),
            });
        }
        let config = ScoringConfig::default();
        let reports = score_attesters(&events, &profiles, &config);

        assert_eq!(reports.len(), 10);
        for r in &reports {
            assert_eq!(
                r.severity,
                Severity::Low,
                "false positive on {}: {:?}",
                r.attester,
                r.reasons
            );
        }
    }

    #[test]
    fn fraudulent_population_is_flagged_high_with_reasons() {
        let mut events = Vec::new();
        let mut profiles = Vec::new();
        for i in 0..10 {
            let attester = format!("honest-{i}");
            events.extend(honest_attester_events(
                &attester,
                "region-a",
                i as u64 * 1000,
                30,
            ));
            profiles.push(AttesterProfile {
                attester,
                region: "region-a".to_string(),
            });
        }
        for i in 0..3 {
            let attester = format!("fraud-{i}");
            events.extend(fraudulent_attester_events(&attester, "region-a"));
            profiles.push(AttesterProfile {
                attester,
                region: "region-a".to_string(),
            });
        }

        let config = ScoringConfig::default();
        let reports = score_attesters(&events, &profiles, &config);

        let fraud_reports: Vec<&RiskReport> = reports
            .iter()
            .filter(|r| r.attester.starts_with("fraud-"))
            .collect();
        assert_eq!(fraud_reports.len(), 3);
        for r in &fraud_reports {
            assert_eq!(
                r.severity,
                Severity::High,
                "fraud attester not flagged high: {}: score {} reasons {:?}",
                r.attester,
                r.score,
                r.reasons
            );
            assert!(
                r.reasons.len() >= 3,
                "expected multiple independent signals for {}",
                r.attester
            );
        }

        let honest_reports: Vec<&RiskReport> = reports
            .iter()
            .filter(|r| r.attester.starts_with("honest-"))
            .collect();
        assert_eq!(honest_reports.len(), 10);
        for r in &honest_reports {
            assert_eq!(
                r.severity,
                Severity::Low,
                "false positive on {}: {:?}",
                r.attester,
                r.reasons
            );
        }

        // Precision/recall over this synthetic population, at the default
        // config: every fraudulent attester flagged, zero honest attesters
        // flagged.
        let true_positives = fraud_reports
            .iter()
            .filter(|r| r.severity != Severity::Low)
            .count();
        let false_positives = honest_reports
            .iter()
            .filter(|r| r.severity != Severity::Low)
            .count();
        let precision = true_positives as f64 / (true_positives + false_positives) as f64;
        let recall = true_positives as f64 / fraud_reports.len() as f64;
        assert_eq!(precision, 1.0);
        assert_eq!(recall, 1.0);
    }

    #[test]
    fn collusion_pair_detected_for_overlapping_record_sets() {
        let shared_hashes: Vec<[u8; 32]> = (0..6).map(hash_for).collect();
        let mut events = Vec::new();
        for (i, hash) in shared_hashes.iter().enumerate() {
            events.push(AttestationEvent {
                attester: "colluder-a".to_string(),
                record_hash: *hash,
                record_region: "region-a".to_string(),
                ledger: i as u32,
                timestamp_secs: i as u64 * 1000,
                revoked: false,
            });
            events.push(AttestationEvent {
                attester: "colluder-b".to_string(),
                record_hash: *hash,
                record_region: "region-a".to_string(),
                ledger: i as u32,
                timestamp_secs: i as u64 * 1000,
                revoked: false,
            });
        }
        // An unrelated attester with no overlap.
        events.push(AttestationEvent {
            attester: "unrelated".to_string(),
            record_hash: hash_for(999),
            record_region: "region-a".to_string(),
            ledger: 0,
            timestamp_secs: 0,
            revoked: false,
        });

        let config = ScoringConfig::default();
        let pairs = find_collusion_pairs(&events, &config);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].attester_a, "colluder-a");
        assert_eq!(pairs[0].attester_b, "colluder-b");
        assert_eq!(pairs[0].shared_record_count, 6);
    }

    #[test]
    fn no_collusion_pair_below_threshold() {
        let config = ScoringConfig::default();
        let events = vec![
            AttestationEvent {
                attester: "a".to_string(),
                record_hash: hash_for(1),
                record_region: "region-a".to_string(),
                ledger: 0,
                timestamp_secs: 0,
                revoked: false,
            },
            AttestationEvent {
                attester: "b".to_string(),
                record_hash: hash_for(1),
                record_region: "region-a".to_string(),
                ledger: 0,
                timestamp_secs: 0,
                revoked: false,
            },
        ];
        assert!(find_collusion_pairs(&events, &config).is_empty());
    }

    #[test]
    fn attester_with_no_flags_is_low_severity_with_zero_score() {
        let events = honest_attester_events("solo", "region-a", 0, 5);
        let profiles = vec![AttesterProfile {
            attester: "solo".to_string(),
            region: "region-a".to_string(),
        }];
        let reports = score_attesters(&events, &profiles, &ScoringConfig::default());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].score, 0.0);
        assert_eq!(reports[0].severity, Severity::Low);
        assert!(reports[0].reasons.is_empty());
    }
}
