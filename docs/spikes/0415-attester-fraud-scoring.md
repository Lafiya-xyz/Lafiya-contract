# Spike: Detecting Fraudulent or Anomalous Attestation Patterns

Tracks [#415](https://github.com/Lafiya-xyz/Lafiya-contract/issues/415). Prototype scoring
lives in [`crates/lafiya-fraud-scoring`](../../crates/lafiya-fraud-scoring); the operational
process it feeds is in
[`docs/runbooks/attester-fraud-response.md`](../runbooks/attester-fraud-response.md).

## Problem

Paying CHWs per verified registration (M2) creates a financial incentive to fabricate
attestations. The allowlist (`attester-registry`) proves an attester is *licensed*; it says
nothing about whether a given attestation reflects a real verification of a real patient.
Human review of every attestation doesn't scale, but indexed on-chain data plus a coarse
off-chain region join can raise the right flags for a human reviewer to act on.

## Features

Six signals, each independently explainable (a reviewer never has to reverse-engineer why an
attester scored the way it did -- every point contributing to a score carries a plain-English
reason string):

| Feature | What it catches | How it's computed |
| --- | --- | --- |
| **Velocity** | Physically implausible attestation rates (the issue's own example: 200 verifications in an hour) | Busiest 1-hour bucket of an attester's timestamps, flagged above an absolute implausibility threshold |
| **Burst timing** | Scripted submission rather than field work | Longest run of attestations whose ledgers are within a small gap of each other |
| **Re-attestation churn** | Repeated attestation of the same record (also a griefing-adjacent pattern -- see the history-eviction discussion in the attestation-registry docs) | Max repeat count of any single `record_hash` by one attester |
| **Region mismatch** | An attester operating (or claiming to operate) far outside their licensed area | Fraction of an attester's attestations whose *record's* off-chain region differs from the attester's own registered region |
| **Revocation ratio** | Attesters whose work is disproportionately walked back later | An attester's revocation rate vs. the **scored population's own mean** revocation rate (peer-relative, not a fixed number -- see "Thresholds") |
| **Collusion graphs** | Clusters of attesters attesting overlapping record sets unusually often | Pairwise shared-`record_hash` counts across all attesters, thresholded (see "Scope" below for why pairwise, not full clustering) |

Each triggered feature contributes `+1.0` to a per-attester score; `severity` is
`High`/`Medium`/`Low` by score threshold. This is a sum of independent binary flags rather than
a trained/weighted model deliberately -- see "Why not a learned model" below.

### Scope: pairwise collusion, not graph clustering

The issue asks for "clusters of attesters who attest overlapping record sets unusually
often." The prototype (`find_collusion_pairs`) computes pairwise shared-record counts, which
already surfaces the clearest 2-attester collusion cases and is trivial for a reviewer to
verify (two named attesters, N shared records, done). Extending pairwise overlap into actual
graph clustering (e.g. connected components over a thresholded overlap graph, or community
detection for larger rings) is a real gap this spike leaves open -- flagged here rather than
half-implemented, since a wrong clustering result is worse than an honest "not built yet."

## Thresholds

Every threshold in `ScoringConfig::default()` is the issue's own illustrative starting point
(e.g. the "200 verifications in an hour" example, rounded down to a still-generous 50/hour for
the default), **not a calibrated production value**. Calibrating them requires:

1. A sample of confirmed-honest CHW attestation timelines (from an early low-stakes
   deployment region, before M2 payment goes live) to measure what "normal" velocity, region
   spread, and revocation rate actually look like -- there is no such dataset yet.
2. Per-region baselines rather than one global number: population density and clinic
   staffing vary enough that a single global velocity cap will either miss dense-region fraud
   or false-positive on legitimately busy dense-region attesters. `ScoringConfig` takes an
   absolute cap today; a per-region baseline (`HashMap<Region, f64>`) is a straightforward
   follow-up once real regional data exists to populate it with.
3. A held-out validation pass any time a threshold changes, using the same synthetic-data
   test harness described below plus, once available, the real confirmed-honest sample from
   (1) as a regression check against false positives.

### Why not a learned model

A trained classifier was considered and rejected for v1: it would need labeled fraud examples
that don't exist yet (fraud, almost by definition, is not yet confirmed in this system's
history), and an opaque model score is a worse fit for the "explainable reasons" deliverable
than a sum of named, independently-reviewable signals. Revisit once (a) a labeled fraud/no-fraud
history exists from this scoring running in production, and (b) the false-positive cost from
the simple sum has actually shown up as a problem.

## False-positive handling

- **Every reason is named and human-readable** (`RiskReport::reasons`) -- a reviewer sees
  exactly which of the six signals fired and the numbers behind it, not just a score.
- **Severity, not a binary flag.** `Medium` (one signal) surfaces for review without
  triggering the "review -> suspend" path automatically; only `High` (multiple independent
  signals) does. A single false-positive signal (e.g. one attester legitimately covering two
  adjacent regions during a staffing gap) lands at `Medium`, not an automatic suspension.
- **Revocation ratio is peer-relative**, not an absolute number, specifically so a
  system-wide revocation-rate change (e.g. a policy change that increases revocations
  everywhere) doesn't spuriously flag every attester at once.
- **The synthetic-data test is the regression gate.** `crates/lafiya-fraud-scoring/src/lib.rs`
  includes `honest_population_is_not_flagged` and
  `fraudulent_population_is_flagged_high_with_reasons`, which construct a synthetic honest
  population (spread-out timing, distinct records, correct region, no revocations) and a
  synthetic fraudulent population (the issue's own failure modes, exaggerated), then assert
  **zero false positives and zero false negatives** (precision = recall = 1.0) against that
  synthetic population at the default config. This is a repeatable check that a future
  threshold change doesn't silently regress either direction -- it is not, and cannot be, a
  claim about real-world precision/recall, which needs the real-data validation pass in
  "Thresholds" above.

## Privacy review

No patient data enters this pipeline, at any stage:

- `record_hash` is the same on-chain commitment `attestation-registry` already stores (see
  [ADR-0001](../adr/0001-hash-only-on-chain-footprint.md), "hash-only on-chain footprint") --
  never a patient identifier or record content.
- `record_region` and an attester's own `region` are coarse (state/province-level, matching
  what `lafiya-web`'s off-chain join already exposes for other purposes) -- never a precise
  location, and never patient-specific.
- `attester` is the attester's own public address, already public via the on-chain allowlist
  -- not patient information.
- `ledger` and `timestamp_secs` are transaction metadata, already public on-chain.
- `revoked` is the same public on-chain boolean `attestation-registry` exposes today.

Nothing in `AttestationEvent` or `AttesterProfile` can be joined back to a specific patient.
The scoring output (`RiskReport`) is about **attesters**, not patients, by construction --
there is no code path in this crate that could emit a patient-identifying report even by
mistake, because no patient-identifying field is ever read in.

## Serving and alerting

The design in the issue calls for this to be "served through the indexer API and fed into the
watchdog as high-severity alerts." Neither `lafiya-indexer` nor a watchdog service exist in
this repository yet -- there's no `crates/lafiya-indexer` to add an analytics module to, and
no watchdog contract or service described elsewhere in this repo -- so that wiring is not
part of this change -- `crates/lafiya-fraud-scoring` is written as a small, dependency-free
library specifically so it can be embedded in either without dictating that service's own
architecture once it exists. The natural integration point, once an indexer service exists: run
`score_attesters`/`find_collusion_pairs` over its stored events on a schedule, and forward any
`Severity::High` report as an alert.

## Deliverables status

- [x] Design doc (this file), covering features, thresholds, false-positive handling, and a
      privacy review.
- [x] An implementation (`crates/lafiya-fraud-scoring`) with synthetic-data tests generating
      honest and fraudulent populations and asserting precision/recall against them.
- [x] An operational playbook (`docs/runbooks/attester-fraud-response.md`): flag -> review ->
      suspend with a reason code -> optional compromise marking.
- [ ] Wiring into a live indexer API and watchdog -- blocked on those services existing (see
      "Serving and alerting").
- [ ] Graph clustering beyond pairwise collusion overlap (see "Scope" above).
- [ ] Threshold calibration against real (not synthetic) attestation data (see "Thresholds"
      above).
