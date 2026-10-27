# CI secrets hygiene

Names only — never values. `scripts/ci/check_secrets_inventory.py` fails CI if a
workflow references a secret that is not listed here.

## Inventory

| Secret | Owner | Scope / used by | Rotation | OIDC / `GITHUB_TOKEN` possible? |
| --- | --- | --- | --- | --- |
| `GITHUB_TOKEN` | GitHub (automatic) | `dashboard.yml`, `docs.yml`, deploy/PR automation; per-job `permissions:` | per run | already used |
| `TESTNET_DEPLOYER_SECRET` | Release maintainers | Stellar **testnet** deployer key: `deploy-testnet.yml`, `smoke-test.yml`, `testnet-reset.yml` | 90 days, and after any maintainer offboarding | **No** — Stellar has no OIDC federation. Testnet-only, holds only friendbot funds; protect with a `testnet` environment and required reviewers. |
| `LAFIYA_WEB_DISPATCH_TOKEN` | Release maintainers | Fine-grained token, `contents:write` on `lafiya-web` only, for `repository_dispatch` in `testnet-reset.yml` | 90 days | **No** — `GITHUB_TOKEN` cannot reach another repository. Preferred replacement: a GitHub App installation token (short-lived) limited to `lafiya-web`. |

There are no registry or cloud credentials in workflows today. If npm/crates.io
publishing is added, use npm trusted publishing / crates.io trusted publishing
(OIDC, `id-token: write` on that job only) rather than a stored token.

## Policy

- Use `GITHUB_TOKEN` or OIDC wherever they suffice; a personal access token is
  never acceptable where either works. Every workflow declares least-privilege
  `permissions:`.
- Secrets are scoped to the narrowest environment (`testnet`), never mainnet keys
  in repository secrets. Mainnet keys follow the ceremony in
  `docs/security/chw-key-custody.md` and are not held by CI.
- Fork PRs never receive secrets; `pull_request_target` is not used with checkout
  of untrusted code.
- Rotate on the cadence above, on suspected exposure, and on offboarding.

## Leak drill (run quarterly and after policy changes)

1. Treat `TESTNET_DEPLOYER_SECRET` as exposed. Disable the `deploy-testnet`,
   `smoke-test`, and `testnet-reset` workflows.
2. Generate a new key (`stellar keys generate --global ci-deployer-new`), fund it
   via friendbot.
3. Transfer admin of deployed testnet contracts to the new key following
   `docs/runbooks/contract-upgrade.md`; verify with `get_admin`.
4. Update the repository secret, re-enable the workflows, run `smoke-test`.
5. Revoke the old key's roles; search workflow logs for the old public key.
6. Record date, time-to-recover, and gaps in the drill log below.

| Date | Secret | Time to recover | Gaps found |
| --- | --- | --- | --- |
| _pending first drill_ | | | |
