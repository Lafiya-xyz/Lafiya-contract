# Capacity and Cost Forecast (national-scale rollout)

Parameterized model turning programme targets into network cost and
operational limits. Reproducible: the only inputs are
[`budgets/capacity-model.json`](../budgets/capacity-model.json) (scenario +
assumptions, versioned via `model_version`) and the per-operation fees in
[`budgets/baseline.json`](../budgets/baseline.json) (see
[storage-cost.md](storage-cost.md)).

```sh
python3 scripts/capacity_model.py          # forecast + top-3 cost drivers
python3 scripts/capacity_model.py --json   # machine-readable
python3 scripts/capacity_model.py --check  # compare against testnet_observations
python3 -m unittest discover -s scripts -p 'test_capacity_model.py'
```

Measured inputs come from the populated-state benchmarks in
`crates/lafiya-bench` (run by the bench-history workflow). Those benchmarks
seed realistic state — 10k attesters, 1M attestations written directly through
testutils, 500 facilities, and full histories — and then measure the hot paths,
so fees and per-entry sizes reflect production-sized state rather than empty
contracts.

## Inputs

| Input | Meaning |
|---|---|
| `cards` | Live cards (default 10,000,000) |
| `registrations_per_day` / `verifications_per_day` | Write (`attest`) and read (`get_attestation`) volume |
| `attesters` | Allowlisted attesters |
| `history_window` | Attestation entries retained per card |
| `ttl_days` | TTL policy; entries are extended once per period |
| `peak_to_average`, `rpc_calls_per_verification` | RPC load shape |

## Outputs

Storage bytes, yearly rent, yearly fee budget, indexer size, peak RPC QPS.

## Model

- Storage = `cards × history_window × (attestation_entry_bytes + overhead)` + attester entries.
- Rent/year = storage × `ttl_ledgers` × `rent_stroops_per_byte_per_ledger` × (365 / `ttl_days`).
- Fees/year = 365 × (registrations × `attest` fee + verifications × `get_attestation` fee).
- Indexer = retained attestations × bytes per event × overhead factor.
- Peak QPS = (verifications × calls each + registrations × 2) / 86,400 × `peak_to_average`.

## Assumptions and limits

- Per-operation fees come from `budgets/baseline.json`, which are **native
  test estimates, not measured testnet fees** (see storage-cost.md).
- `rent_stroops_per_byte_per_ledger` and entry sizes are **placeholders** until
  calibrated; absolute rent figures must not be quoted externally before then.
- **Populated-state measurements:** the benchmark results are fed into the
  capacity model as the measured per-operation fees and entry sizes, replacing
  the synthetic placeholders. When a measured value is available it takes
  precedence over the corresponding assumption in
  `budgets/capacity-model.json`; the source of each input is reported so
  placeholder-derived figures remain identifiable.
- **Validation:** record campaign results in `testnet_observations`
  (`{"operation", "observed_fee_stroops"}`); `--check` fails when any
  prediction deviates more than `tolerance` (default 25%). No observations are
  recorded yet, so the model is not yet validated against testnet.

## Growth assertions

Operations that touch collections are benchmarked across increasing state
sizes, and the benchmark asserts the claimed growth class so superlinear
behavior is caught rather than hidden by tiny state:

| Operation | Collection touched | Claimed growth |
|---|---|---|
| `attest` | card history | O(1) |
| `get_attestation` | card history | O(1) |
| co-attester lookup | co-attesters | O(log n) |
| affiliation check | affiliation periods | O(log n) |
| rate limiting | rate windows | O(1) |

A failure of any assertion fails the bench-history workflow, so the capacity
model is only fed values whose scaling matches the documented claims.

## Sensitivity

Each numeric input is raised 10% and the change in total yearly cost is
reported. With the default scenario the top drivers are the
rent rate, `cards`, and `history_window`, which scale rent linearly (rent
dominates fees). Reducing `history_window` or `ttl_days`-driven extension
frequency is the main lever once calibrated.
