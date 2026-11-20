# Runbook: Relayer and Sponsor Account Operations

Covers the sponsored-attestation relayer
([design](../relayer-sponsored-attestations.md),
[threat model](../security/relayer-threat-model.md)) and fee bumping
([fee-bump guide](../fee-bump.md)).

## Sponsor key custody

- The sponsor key pays fees only. It holds no admin, registrar, or attester
  role on any contract; never grant it one.
- Keep the **operating** sponsor account lightly funded (about a week of
  capped spend) and top it up from a cold treasury account. A key compromise
  then loses at most that float.
- Store the key in a KMS/HSM-backed signer (cloud KMS with ed25519 support or a
  hardware signer). The relayer process requests signatures; it never holds the
  raw seed. Do not place the seed in environment variables, images, or logs.
- Restrict signing to the relayer's service identity; log every signing
  request with the transaction hash.
- Rotate on a schedule (quarterly) and immediately on suspected exposure:
  create a new sponsor account, fund it, deploy the new key to the signer,
  drain the old account to treasury, then retire the old key. Attesters are
  unaffected, because they never sign the outer transaction.
- Separate keys per environment. Testnet keys must never be reused on mainnet.

## Channel-account pool

Concurrent sponsored submissions must not share a source account: two
transactions built from one account at the same time carry the same sequence
number and one is rejected with `txBadSeq`. The relayer therefore leases a
**channel account** per in-flight submission from
`lafiya_rpc_resilience::ChannelPool`.

- **Pool size.** Size the pool to the peak number of *concurrent* submissions,
  not the request rate: `pool_size = ceil(peak_rps × p99_submit_latency_s) + 1`
  spare. Start with 4 channels on testnet and 8 on mainnet; a submission that
  finds every channel leased or quarantined fails fast with
  `PoolError::Exhausted` rather than queueing behind a shared sequence number.
- **Funding.** Each channel account needs only its own reserve (minimum
  account balance) plus a small float for the fees of the transactions it
  sources. Budget `channels × (reserve + max_fee_per_tx × expected_txs_between_topups)`.
  Channel accounts pay fees only; they hold no contract role, exactly like the
  sponsor account, and are topped up from the sponsor account.
- **Sequence tracking.** Each channel tracks its next sequence number locally
  while leased. On `txBadSeq` the pool re-syncs that channel from RPC
  (`account_sequence`) and retries once. Never hand-edit a channel's sequence
  number; re-sync it instead.
- **Quarantine.** A timeout or lost response leaves the transaction's fate
  unknown. The pool quarantines that channel (it is not leased again) until
  `ChannelPool::resolve` polls the pending hash and sees `Accepted`, `Rejected`,
  or `Expired`. Alert on `ChannelPool::quarantined()` being non-empty for more
  than a few ledgers; if a channel stays quarantined, follow the ambiguous-state
  steps in the [RPC outage runbook](rpc-outage-recovery.md).
- **Idempotency.** The pool is keyed by channel, not by request: the relayer's
  idempotency store still decides whether a request is submitted at all, so a
  retried request never leases a second channel for the same key.

## Monitoring

| Signal | Source | Action |
| --- | --- | --- |
| Balance `Low` | `SponsorMonitor::assess` | Top up within one business day |
| Balance `Critical` (below one worst-case fee) | `SponsorMonitor::assess` | Stop accepting sponsored requests; top up immediately |
| Spend rate above the global cap trend | Relayer spend ledger | Review top spenders; suspend abusive attesters |
| Repeated `KeyReused` or `NotAllowlisted` | Relayer logs | Treat as probing; check for key leakage |

## Incident: suspected sponsor key compromise

1. Pause new sponsorship (stop the relayer) and move remaining funds to
   treasury with the old key if still possible.
2. Rotate the key as above.
3. Reconcile the spend ledger against on-chain fees paid by the old account;
   investigate unexplained entries.
4. Record the incident per [security triage](security-triage.md).

## Audit

The relayer's ledger (`Relayer::ledger`) is append-only: sequence number,
time, idempotency key, attester, transaction hash, fee, and outcome. Export it
daily and reconcile it with the sponsor account's on-chain history.
