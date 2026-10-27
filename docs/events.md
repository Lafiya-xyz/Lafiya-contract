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

### `AttestationWithdrawn`

- **Prefix topics:** `attestation_withdrawn`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `attester` | `Address` | topic_list |

### `AttesterRegistryRepointed`

- **Prefix topics:** `attester_registry_repointed`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous` | `Address` | topic_list |
| `new` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `RateLimitHit`

- **Prefix topics:** `rate_limit_hit`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `retry_after_ledger` | `u32` | data |

### `RateLimitSet`

- **Prefix topics:** `rate_limit_set`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `max_per_window` | `u32` | data |
| `window_ledgers` | `u32` | data |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

## `incentive-pool`

### `WorkItemApproved`
Topics: `work_item_id`. Data: `attester`, `payout_amount`. Emitted once per
item by both `approve_work_item` and `approve_work_items`.

### `PayoutClaimed`
Topics: `work_item_id`, `attester`. Data: `recipient` (bound payout
address), `amount`.

### `PayoutAddressScheduled`
Topics: `attester`. Data: `recipient`, `effective_at`. The new recipient is
used for claims from `effective_at` (now + 48h).

### `AttestationRegistryRepointed`
Topics: `previous` (optional), `new`.

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

### `AttesterAdded`

- **Prefix topics:** `attester_added`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `AttesterInfoUpdated`

- **Prefix topics:** `attester_info_updated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `AttesterReinstated`

- **Prefix topics:** `attester_reinstated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `AttesterRemoved`

- **Prefix topics:** `attester_removed`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `AttesterRevoked`

- **Prefix topics:** `attester_revoked`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |

### `AttesterSuspended`

- **Prefix topics:** `attester_suspended`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |
| `reason` | `Symbol` | data |
| `since` | `u64` | data |

### `Initialized`

- **Prefix topics:** `initialized`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `admin` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |
| `contract_kind` | `Symbol` | data |
| `schema_version` | `u32` | data |

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
