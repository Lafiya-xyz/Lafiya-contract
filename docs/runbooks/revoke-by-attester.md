# Runbook: Revoking All Attestations by a Fraudulent Attester

**Audience:** Lafiya registry admin responding to a detected fraudulent attester.

**Scope:** Revoking all `record_hash` attestations associated with a specific
attester, identified via on-chain `AttestationRecorded` event enumeration.

**CLI command:** `lafiya-cli attestation revoke-by-attester`

---

## Background

The `attestation-registry` contract has only `revoke_attestation(record_hash)` —
one call per hash. There is no on-chain index from attester → hashes. When a CHW
is found to be fraudulent, an admin must:

1. Find all `record_hash` values the attester ever attested.
2. Check each hash for collateral damage (co-attesters who would also lose
   their attestations).
3. Revoke each affected hash in turn.

`lafiya-cli attestation revoke-by-attester` automates steps 1–3.

---

## Pre-revocation checklist

- [ ] **Confirm the attester address.** Double-check the `G...` address of the
  fraudulent attester against your registry records.
- [ ] **Determine the time window.** The Soroban RPC retains events for ~7 days
  (~120,960 ledgers). Decide whether you need to cover a longer period — if so,
  you need an event indexer (not yet in scope).
- [ ] **Check the retention window.** Run with `--dry-run` first to confirm the
  requested range is within the RPC's retention window.
- [ ] **Identify co-attesters.** The command warns about collateral damage before
  executing. Review the warning carefully — revoking a `record_hash` removes
  **all** attestations for it, including those from other legitimate attesters.
- [ ] **Prepare admin credentials.** The revocation requires admin auth
  (`revoke_attestation` is admin-only).

---

## Step 1: Dry run — inspect the plan

```bash
lafiya-cli \
  --network testnet \
  attestation revoke-by-attester \
  --attester GABC...xyz \
  --dry-run \
  --audit-json /tmp/revoke-plan.json
```

Review `/tmp/revoke-plan.json` for:
- `hashes_found` — number of unique hashes to examine.
- `collateral_damage_hashes` — hashes where other attesters would also lose
  their attestations.
- Per-entry `collateral_attesters` — the specific addresses affected.

---

## Step 2: Review collateral damage

For each hash with `collateral_attesters`, decide whether to:

- **Accept**: the co-attesters' attestations are also revoked (may require
  re-attestation by legitimate attesters after the fact).
- **Skip**: leave the hash unrevoked if a co-attester's attestation is the
  important one and the fraudulent attester's contribution is negligible.

Currently, `revoke_attestation` is all-or-nothing per hash. Hash-level selective
revocation (per attester) is a planned follow-up.

---

## Step 3: Execute revocations

```bash
lafiya-cli \
  --network testnet \
  --source admin-identity \
  attestation revoke-by-attester \
  --attester GABC...xyz \
  --since 1234000 \
  --until 1354960 \
  --audit-json /var/log/lafiya/revoke-$(date +%Y%m%d).json \
  --audit-csv  /var/log/lafiya/revoke-$(date +%Y%m%d).csv
```

Flags:
- `--since <ledger>` — earliest ledger (default: RPC retention start).
- `--until <ledger>` — latest ledger (default: latest ledger).
- `--audit-json <path>` — machine-readable report.
- `--audit-csv <path>` — human-readable spreadsheet-compatible report.

The command will:
1. Enumerate `AttestationRecorded` events (paged via `getEvents`).
2. Filter by the specified attester.
3. Print a collateral-damage warning if any hashes have co-attesters.
4. Execute `revoke_attestation` for each hash (with poll-before-retry).
5. Write the audit report.

---

## Step 4: Verify

After the run, confirm:

```bash
# Each revoked hash should return None.
lafiya-cli --network testnet attestation get <record_hash>
```

Also check the attester is still present in the allowlist or has been removed:

```bash
lafiya-cli --network testnet attester is GABC...xyz
```

If the attester should be permanently blocked, remove them:

```bash
lafiya-cli --network testnet --source admin-identity attester remove GABC...xyz
```

---

## Retention window error

If you see:

```
Event retention window for testnet does not cover the requested range ...
```

The requested `--since` ledger is older than the RPC's retention window (~7 days).
Options:

1. Narrow the range to within the last 7 days if that covers the fraud period.
2. Use an event indexer (not yet configured) for longer history.

Never request a range the RPC can't cover — the command refuses rather than
silently returning incomplete results.

---

## Audit report format

The JSON report includes:

```json
{
  "network": "testnet",
  "attester": "GABC...xyz",
  "hashes_found": 20,
  "revoked": 18,
  "skipped": 2,
  "failed": 0,
  "collateral_damage_hashes": 3,
  "entries": [
    {
      "record_hash": "aabbcc...",
      "state": "LatestByTarget",
      "collateral_attesters": [],
      "will_revoke": true,
      "result": { "Revoked": { "tx_hash": "abc123..." } }
    },
    ...
  ]
}
```

---

## Integration test scenario (for CI verification)

See `crates/lafiya-cli/src/revoke.rs` tests for the mock-based unit test.
A local-quickstart integration test using 3 attesters and 20 hashes:

```bash
# Spin up local quickstart
docker run --rm -p 8000:8000 stellar/quickstart --local

# Deploy contracts (see contract-upgrade.md)
lafiya-cli --network local deploy --source deployer --admin GADMIN...

# Add 3 attesters
for addr in GA1... GA2... GA3...; do
  lafiya-cli --network local --source admin attester add $addr
done

# Submit 20 attestations from GA1...
for i in $(seq 1 20); do
  hash=$(printf '%064x' $i)
  stellar contract invoke --id CATTREGISTRY... -- attest \
    --attester GA1... --record_hash $hash
done

# Also submit some from GA2 (collateral victims)
for i in $(seq 1 5); do
  hash=$(printf '%064x' $i)
  stellar contract invoke --id CATTREGISTRY... -- attest \
    --attester GA2... --record_hash $hash
done

# Revoke GA1's attestations
lafiya-cli --network local --source admin \
  attestation revoke-by-attester --attester GA1... \
  --audit-json /tmp/audit.json

# Verify: GA2's non-collateral hashes still exist
lafiya-cli --network local attestation get $(printf '%064x' 10)
```

---

## References

- [ADR-0006: Attestation Revocation Semantics](../adr/0006-attestation-revocation-semantics.md)
- Contract function: `revoke_attestation(record_hash: BytesN<32>)` in
  `contracts/attestation-registry/src/lib.rs`
- Event enumeration: `crates/lafiya-cli/src/revoke.rs`
