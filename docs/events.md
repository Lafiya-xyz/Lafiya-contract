# Contract Event Schemas

**Generated file -- do not hand-edit.** Regenerate with:

```bash
make conformance-update
```

This reference is produced directly from the `contractspecv0` section of
each contract's built Wasm (`stellar contract info interface`), so it
reflects what the deployed artifact actually emits, not just what the Rust
source declares. See [`docs/architecture/event-indexing.md`](architecture/event-indexing.md)
for how these events are consumed.

## `attestation-registry`

### `AdminTransferred`

- **Prefix topics:** `admin_transferred`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous_admin` | `Address` | topic_list |
| `new_admin` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttestationRecorded`

- **Prefix topics:** `attestation_recorded`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `attester` | `Address` | data |
| `timestamp` | `u64` | data |
| `sequence` | `u64` | data |
| `evicted_sequence` | `Option<u64>` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttestationTtlExtended`

- **Prefix topics:** `attestation_ttl_extended`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `extended_by` | `Address` | data |
| `previous_expiry` | `u64` | data |
| `new_expiry` | `u64` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `CoAttestationRecorded`

- **Prefix topics:** `co_attestation_recorded`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `attester` | `Address` | topic_list |
| `distinct_attester_count` | `u32` | data |
| `quorum_required` | `u32` | data |
| `quorum_satisfied` | `bool` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `DisputeRaised`

- **Prefix topics:** `dispute_raised`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `disputer` | `Address` | topic_list |
| `timestamp` | `u64` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `DisputeCleared`

- **Prefix topics:** `dispute_cleared`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `cleared_by` | `Address` | topic_list |
| `reason` | `Symbol` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `AttestationRevoked`

- **Prefix topics:** `attestation_revoked`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `by` | `Address` | data |
| `removed_count` | `u64` | data |
| `reason` | `Symbol` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttestationWithdrawn`

- **Prefix topics:** `attestation_withdrawn`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `attester` | `Address` | topic_list |
| `seq` | `u64` | data |

### `AttesterRegistryRepointed`

- **Prefix topics:** `attester_registry_repointed`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous` | `Address` | topic_list |
| `new` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `RateLimitHit`

- **Prefix topics:** `rate_limit_hit`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `retry_after_ledger` | `u32` | data |
| `seq` | `u64` | data |

### `RateLimitSet`

- **Prefix topics:** `rate_limit_set`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `max_per_window` | `u32` | data |
| `window_ledgers` | `u32` | data |
| `seq` | `u64` | data |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

## `incentive-pool`

### `WorkItemApproved`
Topics: `work_item_id`. Data: `attester`, `payout_amount`, `asset`,
`approved_by`, `approved_at`. Emitted once per item by both
`approve_work_item` and `approve_work_items`. `approved_by`/`approved_at`
are the values checked by `void_approvals_since` (see below).

### `PayoutClaimed`
Topics: `work_item_id`, `attester`. Data: `recipient` (bound payout
address), `amount`, `asset`.

### `PayoutDisbursed`
Topics: `work_item_id`, `attester`. Data: `recipient` (bound payout
address), `amount`, `asset`, `disbursed_by` (approver). Emitted once per
item by `disburse`; carries the same fields as `PayoutClaimed` so indexers
can treat both identically.

### `PayoutAddressScheduled`
Topics: `attester`. Data: `recipient`, `effective_at`, `push_enabled`. The
new recipient is used for claims from `effective_at` (now + 48h).
`push_enabled` opts the attester into approver-initiated `disburse`; when
false, `disburse` rejects the item with `PushNotEnabled`.

### `AssetAdded`
Topics: `asset`. Data: `decimals`, `added_by`. Emitted by the admin-managed
asset allowlist after the interface probe (decimals + SAC check) succeeds.

### `AssetRemoved`
Topics: `asset`. Data: `removed_by`. New deposits/approvals for the asset
are rejected; existing reservations remain claimable.

### `AssetDeposited`
Topics: `asset`. Data: `depositor`, `amount`, `total_deposited`. All
accounting is keyed by asset (`TotalDeposited(asset)`, `TotalPaid(asset)`,
reservations, caps); there is no cross-asset conversion on-chain.

### `AssetCapSet`
Topics: `asset`. Data: `max_per_attester`, `max_per_identity`,
`epoch_budget`. Caps and drift detection are per asset.

### `ApprovalsVoided`
Topics: `approver`. Data: `since`, `voided_by`. Sets the O(1) watermark
`ApproverRevokedSince(approver) = since`; `claim`/`disburse` reject items
where `approved_by == approver && approved_at >= since` with
`ApprovalVoided`. Reservations are released lazily by permissionless
sweep-style cleanup.

### `IdentityCapEnforced`
Topics: `identity_key`. Data: `attester`, `asset`, `amount`,
`identity_claimed`. Emitted when a payout is charged against
`max_per_identity`; `identity_key` comes from
`attester-registry.get_identity_key(attester)` and follows rotations.

### `AttestationRegistryRepointed`
Topics: `previous` (optional), `new`.

### Claim / disburse flow

`claim(work_item_id)` (attester-authorized) and
`disburse(work_item_ids)` (approver-only, bounded batch) share one guard
function: asset allowlist, per-asset caps, per-identity caps, eligibility,
approval watermark, replay protection, and push opt-in (disburse only).
Each item pays its current payout binding in the asset recorded on the
work item.

### `EpochConfigProposed`
Data: `epoch_length`, `epoch_budget`, `rollover`, `effective_at`.

### `EpochConfigApplied`
Data: `epoch_length`, `epoch_budget`, `rollover`.

Epoch budget views: `get_epoch_config`, `get_pending_epoch_config`,
`get_epoch_approved(epoch_start)`, `get_epoch_remaining`. Epochs start at
multiples of `epoch_length`; with `rollover` the unused budget of the
immediately preceding epoch is added (no compounding).

### Work-item id derivation

`work_item_id = sha256("LAFIYA-WORK-ITEM-v1" || record_hash || attester.to_xdr())`,
where `attester.to_xdr()` is the `ScVal::Address` XDR. Test vectors (attester
`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF`):

| record_hash | work_item_id |
|---|---|
| `00…00` (32 bytes) | `bd45e96a767a88270c215397386afe1b44d7c2e3665a5d237fd6cc91474172cd` |
| `01…01` (32 bytes) | `3558e08302cd12ec063b38813911118ec8710ffabaf02e520ddb9c0872c386ea` |

## `attester-registry`

### `AdminTransferred`

- **Prefix topics:** `admin_transferred`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous_admin` | `Address` | topic_list |
| `new_admin` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttesterAdded`

- **Prefix topics:** `attester_added`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttesterInfoUpdated`

- **Prefix topics:** `attester_info_updated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttesterReinstated`

- **Prefix topics:** `attester_reinstated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttesterRemoved`

- **Prefix topics:** `attester_removed`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `AttesterRevoked`

- **Prefix topics:** `attester_revoked`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `seq` | `u64` | data |

### `AttesterSuspended`

- **Prefix topics:** `attester_suspended`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `reason` | `Symbol` | data |
| `since` | `u64` | data |
| `seq` | `u64` | data |

### `Initialized`

- **Prefix topics:** `initialized`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `admin` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
| `seq` | `u64` | data |

### `Upgraded`

- **Prefix topics:** `upgraded`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `new_wasm_hash` | `BytesN<32>` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

## `status-list`

### `SegmentUpdated`

- **Prefix topics:** `segment_updated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `segment_index` | `u32` | topic_list |
| `version` | `u64` | data |
| `root` | `BytesN<32>` | data |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |
