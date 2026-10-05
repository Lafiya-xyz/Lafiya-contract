# Testnet Deployment Ledger

Append-only record of every testnet deployment made by CI
(`.github/workflows/deploy-testnet.yml`) and by the testnet-reset automation
(`.github/workflows/testnet-reset.yml`). CI appends rows on the
`testnet-ledger` branch; maintainers merge that branch into `main` periodically.

Flaky runs are tracked by filtering this table for a non-`success` result.

| Time (UTC) | Kind | Commit | attester-registry | attestation-registry | Result | Run |
| --- | --- | --- | --- | --- | --- | --- |
