# Transparency dashboard

A static site showing verified-record activity, attester counts, incentive
accounting (once M2 exists), and contract health, computed only from public
contract events. Every table links to CSV and JSON downloads and to
`data/provenance.json`, which states the network, ledger range, contract IDs,
the exact `getEvents` query, and a command that recomputes it.

Privacy controls (k = 10, weekly × state-level cells) are documented in
[PRIVACY.md](PRIVACY.md).

## Generate locally

```sh
# 1. Export events for a ledger range (see docs/runbooks/watchdog.md for watchdog.toml)
lafiya-watchdog --config watchdog.toml export --from 1000000 --to 1100000 > events.jsonl

# 2. Map attester -> state-level region, from get_attester_info for each attester:
#    stellar contract invoke --id <attester-registry> --network testnet -- get_attester_info --attester G...
echo '{"GABC...": "lagos"}' > attesters.json

# 3. Build the site
python3 dashboard/generate.py --events events.jsonl --attesters attesters.json \
  --manifest docs/release-manifest/examples/v0.1.0-dev.json \
  --network testnet --ledger-from 1000000 --ledger-to 1100000 \
  --contracts C...,C... --out site
```

## Tests and reproducibility

```sh
python3 -m unittest discover -s dashboard
```

CI regenerates `fixtures/expected/` from the pinned synthetic range in
`fixtures/events.jsonl` (created by `fixtures/make_synthetic.py`) and fails on
any difference. After an intentional output change, regenerate with the
arguments in `test_generate.py` (`ARGS`) and commit the new expected files.

## Deployment

`.github/workflows/dashboard.yml` runs daily, appends new events to
`dashboard/events.jsonl` on the `gh-pages` branch (so history outlives RPC event
retention), regenerates the site, and publishes it under `/dashboard`. It runs
only when the repository variables `LAFIYA_DASHBOARD_NETWORK`,
`LAFIYA_DASHBOARD_RPC_URL`, `LAFIYA_DASHBOARD_CONTRACTS`, and
`LAFIYA_DASHBOARD_START_LEDGER` are set. The attester region map is read from
`config/attester-regions.<network>.json`.
