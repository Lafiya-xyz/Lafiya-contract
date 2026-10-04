# Runbook: Testnet Reset

SDF periodically resets testnet and futurenet, wiping every account and
contract. After a reset every contract ID in `config/networks.toml`, the
bindings, and `lafiya-web`'s environment points at nothing.

## Automation

`.github/workflows/testnet-reset.yml` runs hourly:

1. `scripts/testnet_reset/detect.py` classifies the network as
   `healthy | contract-missing | network-reset`:
   - `getLatestLedger` vs. the baseline in `config/testnet-state.json`: a drop
     below 50 % of the recorded sequence (or a changed `getNetwork` passphrase)
     is a **network-reset**;
   - `getLedgerEntries` on each configured contract instance: any missing
     instance on an un-reset network is **contract-missing** (usually TTL expiry).
2. On `network-reset` the `redeploy` job (in the `testnet` environment):
   1. funds the deployer through friendbot and redeploys the canonical system
      with `scripts/deploy-testnet.sh` (signer from the `TESTNET_DEPLOYER_SECRET`
      environment secret);
   2. re-seeds a demo attester and attestation (`seed-fixtures.sh`);
   3. opens a PR updating `config/networks.toml`, any bindings `networks`
      constants, `config/testnet-state.json`, and the
      [deployment ledger](../deployments/testnet.md) (`reconcile.py`);
   4. sends `repository_dispatch` (`lafiya-testnet-reset`) to `lafiya-web`
      when `LAFIYA_WEB_DISPATCH_TOKEN` is configured, and opens an issue to
      notify maintainers.
3. On `contract-missing` it opens an alert issue; no automatic redeploy.

## Manual steps

- **Review and merge the automated PR** promptly; consumers are broken until then.
- **contract-missing:** check whether the instance TTL lapsed; restore with
  `stellar contract restore` or redeploy manually and run `reconcile.py`.
- **False positive:** if `detect.py` misfires, re-baseline with
  `python3 scripts/testnet_reset/detect.py --update-state` and commit
  `config/testnet-state.json`.

## Verification

- Unit tests (fixture RPC responses for every state):
  `python3 -m unittest discover -s scripts/testnet_reset`
- End-to-end dry run against a restarted local quickstart:
  `scripts/testnet_reset/dry-run-local.sh`
- Automatic PR: trigger the workflow manually after deleting
  `config/testnet-state.json` on a test branch and setting the baseline
  sequence artificially high.

## Guidance for consumers

Do not hard-code testnet contract IDs. Load them at build or run time from the
published release manifest (`release-manifest.json`, see
[ADR-0010](../adr/0010-release-manifest-and-compatibility.md)) or from
`config/networks.toml` on `main`, and subscribe to the
`lafiya-testnet-reset` `repository_dispatch` event to refresh them.
