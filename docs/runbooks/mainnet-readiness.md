# Mainnet readiness gate

Mainnet is disabled while every id under `[mainnet.contracts]` in `config/networks.toml` is empty.
CI (`scripts/check_mainnet_readiness.py`) fails any change that fills one in unless
`config/mainnet-readiness.json` validates. Owner sign-off is enforced by `.github/CODEOWNERS`,
which requires review from the owners of `config/networks.toml`, the manifest, and `audit/`.

## Manifest schema (`config/mainnet-readiness.json`)

```json
{
  "audit_report":        { "path": "audit/reports/<file>", "sha256": "<64 hex>" },
  "ceremony_record":     { "path": "<repo path>", "sha256": "<64 hex>" },
  "game_day_report":     { "path": "<repo path>", "sha256": "<64 hex>" },
  "monitoring_evidence": { "path": "<repo path>", "sha256": "<64 hex>" },
  "owner_signoff":       ["@handle"]
}
```

Each evidence `path` must be a file inside the repository whose SHA-256 matches `sha256`.
`owner_signoff` is a non-empty list of owner handles; the matching CODEOWNERS approval is what
GitHub enforces on the pull request.

## Release process

1. Complete the audit, key ceremony, rehearsal (game day) and monitoring set-up.
2. Commit each evidence document and record its hash in the manifest.
3. In a separate pull request, fill in the mainnet contract ids; owners review and approve.
4. CI blocks the merge until the manifest validates.

Test: `python3 -m pytest scripts/tests/test_check_mainnet_readiness.py`.
