# Privacy review: published dashboard aggregates

Status: reviewed for the initial dashboard (issue #416). Re-review whenever a
new aggregate, dimension, or finer granularity is added.

## What the chain reveals already

Lafiya stores only record **hashes** on chain (ADR-0001). An
`attestation_recorded` event exposes a record hash, the attester address, and
the ledger time. The attester's `region` is public through `get_attester_info`.
The dashboard publishes nothing that is not already derivable from public
chain data; its risk is making re-identification **easier** by pre-computing
small, fine-grained counts.

## Controls

| Risk | Control |
| --- | --- |
| Small cells single out a patient in a small area and time window | Counts are published per ISO **week** × **state-level region** only. Any cell with 1 to 9 records (k = 10) is suppressed (`<10` in HTML, empty in CSV, `null` in JSON). |
| A suppressed cell is recovered by subtraction | Secondary suppression: if a week has exactly one suppressed cell for a metric, the next-smallest cell is suppressed too. No weekly or regional totals are published. |
| Region finer than state level | Regions come from `AttesterInfo.region`. Operators must register state-level values only (enforced by review of `add_attester_with_info` calls; the dashboard does not publish LGA, facility, or attester-level breakdowns). Unknown regions are pooled into `unknown`. |
| Attester-level activity | Not published. Attester counts are totals by status only. |
| Record hashes or addresses in datasets | Datasets contain no record hashes. Contract health lists admin and contract addresses, which are operational public keys, not patient data. |
| Timing correlation (a patient knows when they were verified) | Weekly buckets; no per-day or per-ledger series. |
| Incentive payouts linked to individuals | When M2 lands, payouts are aggregated to the same week × region cells with the same k threshold; per-transaction links point to incentive-contract transactions, not to records. Re-review required before enabling. |

## Residual risk

- A region with persistently low volume will mostly show `<10`. That is the
  intended trade-off.
- Anyone can still run the open-source exporter on raw chain data. The
  dashboard does not add capability beyond that; it only avoids publishing
  small cells.

## Verification

`python3 -m unittest discover -s dashboard` asserts that no published cell
holds a value between 1 and k−1, and tests secondary suppression.
