# API reference (rustdoc)

The rustdoc for this version is published alongside the site:

- [`attester_registry`](api/attester_registry/index.html)
- [`attestation_registry`](api/attestation_registry/index.html)
- [`incentive_pool`](api/incentive_pool/index.html)
- [`multisig_account`](api/multisig_account/index.html)
- [`lafiya_commitment`](api/lafiya_commitment/index.html)
- [`lafiya_config`](api/lafiya_config/index.html)
- [`lafiya_rpc_resilience`](api/lafiya_rpc_resilience/index.html)

Build it locally with `cargo doc --workspace --no-deps --open`.

## `incentive_pool` entry points

The pool supports multiple allowlisted assets with isolated accounting, an
optional approver-initiated push disbursement mode, approval voiding by
watermark, and per-identity payout caps. All accounting keys are asset-keyed
(`TotalDeposited(asset)`, `TotalPaid(asset)`, reservations, caps); there is no
on-chain cross-asset conversion.

### Asset allowlist

- `add_asset(asset: Address)` — admin-only. Probes the token interface on add
  (decimals, SAC check) before allowlisting. Rejects assets that fail the probe.
- `remove_asset(asset: Address)` — admin-only. Removes an asset from the
  allowlist; existing reservations for that asset remain sweepable.
- `get_asset(asset: Address) -> AssetInfo` — returns allowlist metadata
  (decimals, SAC status) for an asset.
- `list_assets() -> Vec<Address>` — returns the current allowlist.

### Deposits, claims, and push disbursement

- `deposit(asset: Address, from: Address, amount: i128)` — deposits an
  allowlisted asset; enforces the per-asset cap.
- `claim(work_item_id: BytesN<32>)` — attester-authorized claim. Applies the
  per-asset caps, eligibility, replay protection, and approval-void watermark.
- `disburse(work_item_ids: Vec<BytesN<32>>)` — approver-only, bounded batch.
  Pays each item to its current payout binding, applying the same guards as
  `claim` (shared guard function). Only items whose attester opted in to push
  mode are eligible. Emits an audit event per item. The maximum batch size is
  derived from budget measurements and documented in the contract rustdoc.

### Payout binding and push opt-in

- `bind_payout(attester: Address, payout: Address, push_enabled: bool)` — binds
  the recipient address and opts the attester in or out of push mode. An
  approver cannot push to an address the CHW never chose.
- `get_payout(attester: Address) -> PayoutBinding` — returns the bound payout
  address and push-mode flag.

### Approval voiding

- `void_approvals_since(approver: Address, since: u64)` — admin or guardian
  only. Sets an O(1) watermark `ApproverRevokedSince(approver) = since`. Claims
  of items where `approved_by == approver && approved_at >= since` fail with a
  typed error. Items from other approvers, or approved before `since`, are
  unaffected.
- `sweep_expired(work_item_ids: Vec<BytesN<32>>)` — permissionless cleanup that
  releases reservations for voided or expired items lazily.

### Per-identity caps

- `max_per_identity` — enforced alongside `max_per_attester` when the
  attester-registry exposes identity keys. Cumulative totals follow rotations,
  so rotated keys keep the same identity. Falls back to per-address behavior
  when the registry does not support identity keys; the capability is
  negotiated through `get_interface()`.

## `attester_registry` additions

- `get_identity_key(attester: Address) -> BytesN<32>` — returns a stable
  identity key derived from the canonical licence hash. Follows rotations so
  rotated keys keep the same identity. The pool uses this to enforce
  `max_per_identity` across multiple attester addresses.
- `get_interface()` — advertises the `identity_key` feature so older registries
  without it still work with the pool's per-address fallback.