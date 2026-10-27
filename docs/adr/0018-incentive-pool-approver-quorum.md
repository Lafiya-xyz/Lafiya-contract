# ADR-0018: In-pool M-of-N approver quorum for the incentive pool

- **Status:** Accepted
- **Date:** 2026-10-21
- **Deciders:** Maintainers and relevant reviewers

## Context

`incentive-pool` authorizes payouts with a single `Approver` address. Even if
that address is a `multisig-account`, the pool cannot see independent
approvers or distinct review steps, so watchdogs cannot alert on *who*
endorsed a payout, and one leaked key (or one multisig quorum signing a
batch off-chain) can authorize every payout up to the per-claim cap.

## Decision

Implement the quorum **inside the pool**:

- `set_approver_set(approvers, threshold)` (admin) configures N approvers and
  an M threshold. `threshold = 0` with an empty set keeps **single-approver
  mode** (`approve_work_item` / `approve_work_item_from` with `Approver`),
  which remains the pilot configuration. While a quorum is configured the
  single-approver entry points fail with `QuorumRequired`.
- `propose_work_item(proposer, id, attester, amount, programme)` creates a
  proposal with an expiry (`set_proposal_ttl`, default 7 days); the proposer
  is the first endorsement.
- `endorse_work_item(endorser, id)` adds an endorsement; a repeat endorsement
  fails with `DuplicateEndorsement`, an expired proposal with `ProposalExpired`.
- `execute_work_item(id)` is permissionless. It counts only endorsements by
  addresses in the **current** set against the **current** threshold, so
  removing an endorser mid-proposal or raising the threshold takes effect
  immediately, and re-validates the approval (allowlist, caps, programme
  balance) before writing the work item.
- Every endorsement is charged to the endorser's rate-limit window
  (see `docs/storage-cost.md#incentive-pool`).

Events for watchdog rules: `approver_set_changed` (critical),
`work_item_proposed` and `work_item_endorsed` (topics: work item id,
endorser; data: valid endorsement count and threshold), plus the existing
`work_item_approved` on execution.

## Alternatives considered

### Compose with `multisig-account` as the `Approver`

No contract change, but the endorsements happen inside the multisig and are
invisible to the pool's event stream; per-approver rate limits are
impossible because the pool sees one address; and the multisig's
authorization is unscoped (ADR-0007), so a multisig quorum approval is not
bound to a specific work item review step. Still supported: a multisig can
be set as the single `Approver` or be one member of the approver set.

### Off-chain co-signing service

Rejected: moves the security boundary off-chain and is not auditable.

## Consequences

- Each approval in quorum mode costs M transactions and one transient
  `Proposal` persistent entry (removed on execution).
- Pending proposals survive set changes but are re-evaluated at execution.
- Tests must cover threshold change, endorser removal mid-proposal, and
  duplicate endorsement.
