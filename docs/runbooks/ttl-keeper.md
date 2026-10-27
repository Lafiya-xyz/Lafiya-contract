# Runbook: TTL keeper (`scripts/ops/ttl_keeper.py`)

TTL policies live in contract code, but rarely-written instance and persistent entries still
archive unless someone extends them. The keeper scans watched entries over RPC and extends any
whose remaining TTL is below a threshold, within a spend cap. See [storage cost](../storage-cost.md).

## Configuration

```json
{
  "source_identity": "lafiya-keeper",
  "extend_below_ledgers": 518400,
  "alert_below_ledgers": 172800,
  "extend_to_ledgers": 2073600,
  "daily_cap_stroops": 5000000,
  "entries": [
    { "id": "attester-registry-instance", "contract": "C...", "key_xdr": "<base64 LedgerKey>",
      "est_cost_stroops": 100000 }
  ]
}
```

Run: `python3 scripts/ops/ttl_keeper.py --config keeper.json --rpc-url <url> [--dry-run]`
(schedule every few hours from cron or a supervised timer). Output is JSON lines
(`extended`, `would_extend`, `cap_reached`, `ALERT`); exit code `2` means an entry is below
`alert_below_ledgers` and needs attention.

## Behavior

- **Dry run:** `--dry-run` logs `would_extend` and submits nothing.
- **Spend cap:** estimated (or reported) cost per extension is added to a per-UTC-day ledger in
  the state file; an extension that would exceed `daily_cap_stroops` is skipped and logged as
  `cap_reached`, and the entry keeps alerting.
- **Idempotent / multi-instance:** the state file is held under an exclusive lock and each entry's
  TTL is re-read under the lock, so a second instance sees the first one's extension and does
  nothing. Instances must share the state file (shared volume) for the spend cap to be global.
- **Failure behavior:** an RPC error aborts the pass without extending anything (fail safe) and
  the process exits non-zero; an entry missing from RPC is reported as possibly archived, which
  needs a restore operation (not automated). Extension failures from the CLI surface as errors.
  Pair the job with a dead-man's switch so a silent keeper pages on-call.

## Key custody

The keeper never reads secret keys. `source_identity` names a `stellar keys` identity on the host.
Use a dedicated low-balance account that only pays fees (extension needs no contract authority),
fund it to roughly a few days of cap, and keep it separate from admin and attester keys.
Rotate by creating a new identity and updating `source_identity`.

Test: `python3 -m pytest -o addopts="" scripts/tests/test_ttl_keeper.py` (includes a simulated
archival scenario, cap enforcement, and concurrent instances).
