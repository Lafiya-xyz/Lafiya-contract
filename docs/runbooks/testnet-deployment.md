# Runbook: Continuous Testnet Deployment

Workflow: `.github/workflows/deploy-testnet.yml`. Smoke scenario:
`.github/workflows/smoke-test.yml` (reusable) → `scripts/smoke-test.sh`.

## What runs when

| Trigger | Job | Effect |
| --- | --- | --- |
| Push to `main` touching `contracts/**` | `ephemeral` → `smoke` → `record` | Fresh deploy with new contract IDs (stellar-cli uses a random salt per deploy), smoke scenario, ledger row. |
| Nightly (03:17 UTC) | same | Same; proves `main` still deploys and works on a real network. |
| Tag `v*` | `staging-upgrade` | Upgrades (never redeploys) the long-lived staging contracts with `scripts/upgrade.sh`, exercising the upgrade path on real state. |
| Manual dispatch | `ephemeral` → `smoke` → `record` | On-demand run. |

Contract IDs and the smoke result are posted to the job summary and appended to
[`docs/deployments/testnet.md`](../deployments/testnet.md) via the
`testnet-ledger` branch.

## Secrets policy

- **No workflow accepts secrets as inputs.** `workflow_dispatch` inputs are stored
  in the run's event payload and are readable through the API, even when masked
  in logs. `smoke-test.yml` no longer has an `admin_secret` input.
- The deployer/admin key is the **environment secret** `TESTNET_DEPLOYER_SECRET`
  on the `testnet` GitHub Environment. It is never a repository-level secret.
- **Rotate any key that was ever passed as a workflow input.** Generate a new
  key, fund it with friendbot, move admin rights with `propose_admin` /
  `accept_admin` on any long-lived contract it administers, then replace the
  environment secret.

## Environment protection configuration

Settings → Environments:

| Environment | Setting | Value |
| --- | --- | --- |
| `testnet` | Deployment branches and tags | `main` and tags matching `v*` only |
| `testnet` | Secrets | `TESTNET_DEPLOYER_SECRET` |
| `testnet` | Variables | `STAGING_ATTESTER_REGISTRY`, `STAGING_ATTESTATION_REGISTRY` |
| `testnet` | Required reviewers | none (automated nightly runs) |
| `mainnet` (future) | Deployment branches and tags | tags matching `v*` only |
| `mainnet` (future) | Required reviewers | at least 2 maintainers; prevent self-review |
| `mainnet` (future) | Wait timer | 30 minutes |

## Friendbot

The `ephemeral` job funds the deployer through friendbot when its account does
not exist (for example after a testnet reset). If friendbot is unreachable the
step fails with a `Friendbot unavailable` error annotation, which fails the run
and triggers GitHub's failed-workflow notifications to maintainers.

## Triage

- **Smoke failed, deploy succeeded:** a regression on `main`, or testnet
  flakiness. Rerun once; if it fails again, open an issue with the run link.
- **Deploy failed:** check friendbot status, RPC health
  ([rpc-outage-recovery.md](rpc-outage-recovery.md)), and whether testnet was
  reset ([testnet-reset.md](testnet-reset.md)).
- **Staging upgrade failed:** follow [contract-upgrade.md](contract-upgrade.md).
- **Green for a week:** the ledger should show seven consecutive nightly
  `success` rows. Record any flaky run as an issue labelled `flaky`.
