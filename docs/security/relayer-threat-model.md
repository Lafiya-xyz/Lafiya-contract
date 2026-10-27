# Relayer Threat Model: Fee-Drain and Replay

Scope: the sponsored-attestation relayer described in
[relayer-sponsored-attestations.md](../relayer-sponsored-attestations.md) and
implemented as policy in [`crates/lafiya-relayer`](../../crates/lafiya-relayer).
Operations: [relayer runbook](../runbooks/relayer-operations.md).

## Assets

| Asset | Why it matters |
| --- | --- |
| Sponsor account XLM | Pays every sponsored fee; draining it halts attestation for all CHWs |
| Sponsor signing key | Can sign any transaction and spend the balance |
| Attester auth entries | Authorize one `attest` invocation; must not be reusable or redirected |

## Fee-drain attacks

| Threat | Control | Residual risk |
| --- | --- | --- |
| Non-attester submits requests to burn fees | Relayer only sponsors addresses on its allowlist (`allowed_attesters`), mirrored from `attester-registry` | A compromised legitimate CHW key can still spend its own quota |
| Compromised/abusive CHW floods requests | Per-attester quota per time window and lifetime per-attester spend cap | Bounded by cap; suspend the attester on-chain |
| Single transaction with inflated fee | `max_fee_per_tx`; fee-bump escalation bounded by `FeeBumpPolicy.max_fee`/`max_bumps` | Operator misconfiguration |
| Many attesters collude | Global spend cap; sponsor balance alerts (`SponsorMonitor`) | Cap exhaustion is a denial of service, not a loss of funds beyond the cap |
| Transactions that fail on chain still cost fees | Fee is reserved in the ledger from the moment the network may have seen the transaction; failed submissions count against quotas | Intended: failed spam is not free |
| Lost responses used to get a free retry | Ambiguous submits are resolved by polling the hash, never by resubmitting | None |

## Replay attacks

| Threat | Control |
| --- | --- |
| Client retries the same request (network flake) | Idempotency key: same key + same tx returns the recorded outcome with no resubmit and no second charge |
| Same signed envelope sent under a fresh key | Replays are detected by transaction hash and return the stored outcome |
| Idempotency key reused for a different transaction | Rejected with `KeyReused` |
| Reusing a captured auth entry elsewhere | Entries are bound to network, contract, method, arguments, and `signature_expiration_ledger`; the contract rejects expired or mismatched entries, and Stellar sequence numbers make the outer transaction single-use |
| Relayer substitutes the attested record | The attester signs the exact invocation arguments; any change invalidates the signature |

## Out of scope

Compromise of the sponsor key itself (see custody guidance in the runbook),
compromise of the CHW device (see [CHW key custody](chw-key-custody.md)), and
RPC provider dishonesty (see [quorum reads](quorum-reads.md)).
