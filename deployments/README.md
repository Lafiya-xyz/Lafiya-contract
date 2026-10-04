# Deployment ledger

`deployments/<network>.jsonl` is an **append-only** history of what has been deployed,
initialized, upgraded, migrated, repointed, or had its admin transferred, per network. It
exists because `config/networks.toml` only records the *current* contract ID per network —
see [ADR-0010](../docs/adr/0010-release-manifest-and-compatibility.md#deployment-promotion-records)
for the gap this closes.

## Format

One JSON object per line, matching [`schema.json`](schema.json). Fields:

| Field | Meaning |
| --- | --- |
| `event` | `deploy` \| `initialize` \| `upgrade` \| `migrate` \| `admin_transfer` \| `repoint` |
| `contract_kind` | `attester-registry` \| `attestation-registry` \| `multisig-account` |
| `contract_id` | The Stellar contract ID this event applies to |
| `wasm_sha256` | sha256 of the wasm now running, or `null` for events that don't change code |
| `previous_wasm_sha256` | sha256 of the wasm this replaced (upgrade/migrate), or `null` |
| `tx_hash` | The confirming transaction's hash, or `null` if it could not be captured |
| `ledger` | Ledger sequence the transaction closed in, or `null` |
| `timestamp` | RFC 3339 UTC timestamp the record was appended |
| `git_commit` | `git rev-parse HEAD` at the time of the event |
| `release_version` | `[workspace.package].version` at the time of the event |
| `operator` | Identity/signer-set that authorized the event |
| `prev_record_sha256` | sha256 of the previous line in this file, or `null` for the first record |

`prev_record_sha256` makes the file a hash chain: editing or deleting any line invalidates
every record after it. `lafiya-cli deployments verify` detects this.

## Writing records

Records are appended automatically:

- `scripts/deploy.sh` and `scripts/upgrade.sh` call `lafiya-cli deployments record` after a
  `stellar contract deploy`/`invoke`/`upgrade` confirms, using the tx hash the `stellar` CLI
  prints. If the CLI's output format changes and the hash can't be parsed, the script warns
  loudly and still appends a record with `tx_hash: null` — the ledger stays honest about
  what it doesn't know, rather than silently skipping the event.
- For a manual/out-of-band change (e.g. `admin_transfer` done directly via `stellar contract
  invoke`), append by hand:

  ```sh
  cargo run -p lafiya-cli -- --network testnet deployments record \
    --event admin_transfer --contract-kind attester-registry \
    --contract-id C... --tx-hash <hash> --operator <identity>
  ```

## Verifying

```sh
cargo run -p lafiya-cli -- --network testnet deployments verify
```

Checks, per network file:

- every record's `prev_record_sha256` matches the sha256 of the line before it (hash chain
  intact — nothing was edited or deleted);
- ledgers are monotonically non-decreasing across the file;
- every record has the fields its `event` requires.

`verify` today only checks the file's internal consistency (hash chain, monotonic ledgers,
required fields), offline. Confirming that the *instance's current on-chain wasm hash*
equals the latest record, and that each `tx_hash` actually exists and succeeded, needs a
live RPC connection; that on-chain cross-check is a tracked follow-up on top of this ledger
rather than something `verify` does today.

## CI

A nightly job (`.github/workflows/deployments-verify.yml`) runs `deployments verify
--network testnet` so hash-chain corruption is caught even between human deploys.
