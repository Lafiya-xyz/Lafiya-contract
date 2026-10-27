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
- **Validation:** record campaign results in `testnet_observations`
  (`{"operation", "observed_fee_stroops"}`); `--check` fails when any
  prediction deviates more than `tolerance` (default 25%). No observations are
  recorded yet, so the model is not yet validated against testnet.

## Sensitivity

Each numeric input is raised 10% and the change in total yearly cost is
reported. With the default scenario the top drivers are the
rent rate, `cards`, and `history_window`, which scale rent linearly (rent
dominates fees). Reducing `history_window` or `ttl_days`-driven extension
frequency is the main lever once calibrated.
