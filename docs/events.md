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

### `AttestationRecorded`

- **Prefix topics:** `attestation_recorded`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |
| `attester` | `Address` | data |
| `timestamp` | `u64` | data |

### `AttestationRevoked`

- **Prefix topics:** `attestation_revoked`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `record_hash` | `BytesN<32>` | topic_list |

### `AttesterRegistryRepointed`

- **Prefix topics:** `attester_registry_repointed`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous` | `Address` | topic_list |
| `new` | `Address` | topic_list |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |

## `attester-registry`

### `AdminTransferred`

- **Prefix topics:** `admin_transferred`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `previous_admin` | `Address` | topic_list |
| `new_admin` | `Address` | topic_list |

### `AttesterAdded`

- **Prefix topics:** `attester_added`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |

### `AttesterReinstated`

- **Prefix topics:** `attester_reinstated`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `attester` | `Address` | topic_list |

### `AttesterRemoved`

- **Prefix topics:** `attester_removed`
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

### `Initialized`

- **Prefix topics:** `initialized`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `admin` | `Address` | topic_list |

### `Paused`

- **Prefix topics:** `paused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |

### `Unpaused`

- **Prefix topics:** `unpaused`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `by` | `Address` | topic_list |

### `Upgraded`

- **Prefix topics:** `upgraded`
- **Data format:** `map`

| Field | Type | Location |
|---|---|---|
| `new_wasm_hash` | `BytesN<32>` | topic_list |
