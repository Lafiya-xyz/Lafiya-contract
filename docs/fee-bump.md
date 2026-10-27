# Fee-Bump Transactions

Implemented in
[`crates/lafiya-rpc-resilience/src/feebump.rs`](../crates/lafiya-rpc-resilience/src/feebump.rs).

## Relation to inclusion-fee bumping in `fees.rs`

[`fees.rs`](../crates/lafiya-rpc-resilience/src/fees.rs) holds the pure
decision: `decide_fee_bump` says whether a transaction pending for N ledgers
should be re-bid and at what inclusion fee, bounded by `--max-fee`.
`feebump.rs` is the driver around it: it submits, polls by hash, calls
`decide_fee_bump`, and submits a sponsor-signed `FeeBumpTransaction` built
from the stored inner transaction by a `FeeBumpBuilder`.

## Why no duplicate effect

A fee bump wraps the original transaction. The inner hash and sequence number
are unchanged, so the transaction can be included at most once; polling by the
inner hash (ADR-0011) tracks the original and bumped submissions together.
Nothing is re-signed from scratch.

## Runaway escalation caps

- `FeeBumpPolicy.max_fee`: a hard ceiling on any bid.
- `FeeBumpPolicy.max_bumps`: maximum number of bumps per intent.
- Sponsor balance: a bump is refused when the sponsor cannot cover the new fee.
- `SponsorMonitor` reports `Low`/`Critical` balance alerts.

## Operator guidance

Sponsor key custody, rotation, and monitoring are in the
[relayer runbook](runbooks/relayer-operations.md#sponsor-key-custody).
