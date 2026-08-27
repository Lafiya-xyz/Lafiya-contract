# Lafiya Smart Contract Error Codes

This document enumerates the error codes defined in the Lafiya Soroban smart contracts.

> [!IMPORTANT]
> **Error codes are contract-scoped, not global.** Each contract defines its own `Error` enum starting from `1`. To correctly interpret an error code, you must know which contract produced the error.

## `attester-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | The contract has not been initialized yet. |
| `2` | `AlreadyInitialized` | The contract is already initialized; double-initialization is rejected. |
| `3` | `NoPendingTransfer` | No admin transfer is pending; `accept_admin` was called before `propose_admin` nominated a successor (admin transfer is a two-step flow: `propose_admin` then `accept_admin`). |
| `4` | `ContractPaused` | The contract is paused; state-changing calls are rejected until an admin calls `unpause`. |
| `5` | `AllowlistFull` | The allowlist has reached its configured soft cap (`set_max_attesters`). |
| `6` | `MigrationNotRequired` | No pending storage migration; `SchemaVersion` is already current. |
| `7` | `AttesterNotFound` | `update_attester_info` was called for an address that is not currently allowlisted (never added, or since removed). |

## `attestation-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | The contract has not been initialized yet. |
| `2` | `AlreadyInitialized` | The contract is already initialized; double-initialization is rejected. |
| `3` | `AttesterNotAllowlisted` | The attester address is not allowlisted in the configured `attester-registry` contract. |
| `4` | `NoPendingTransfer` | No admin transfer is pending. |
| `5` | `InvalidRegistryWiring` | The configured attester-registry address is invalid or unreachable. |
| `6` | `AttestationNotFound` | No attestation exists for the given record hash. |
| `7` | `ContractPaused` | The contract is paused; state-changing calls are rejected until an admin calls `unpause`. |

## `multisig-account`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `InvalidThreshold` | The threshold is zero or exceeds the configured signer count. |
| `2` | `DuplicateSigner` | The signer configuration contains the same public key more than once. |
| `3` | `NotEnoughSigners` | The supplied signature count is below the configured threshold. |
| `4` | `BadSignatureOrder` | Signatures are duplicated or are not strictly ordered by public key. |
| `5` | `UnknownSigner` | A signature belongs to a public key that is not a configured signer. |
| `6` | `NotInitialized` | The account's signer threshold is unavailable. |
| `7` | `TooManySigners` | The supplied signature count exceeds the configured signer count. |
