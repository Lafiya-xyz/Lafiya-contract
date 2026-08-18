# Lafiya Smart Contract Error Codes

This document enumerates the error codes defined in the Lafiya Soroban smart contracts.

> [!IMPORTANT]
> **Error codes are contract-scoped, not global.** Each contract defines its own `Error` enum starting from `1`. To correctly interpret an error code, you must know which contract produced the error.

## `attester-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | The contract has not been initialized yet. |
| `2` | `AlreadyInitialized` | The contract is already initialized; double-initialization is rejected. |
| `3` | `NoPendingTransfer` | No admin transfer is pending. |
| `4` | `ContractPaused` | The contract is paused; state-changing calls are rejected until an admin calls `unpause`. |
| `5` | `AllowlistFull` | The allowlist has reached its configured soft cap (`set_max_attesters`). |
| `6` | `MigrationNotRequired` | No pending storage migration; `SchemaVersion` is already current. |

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

## `incentive-pool`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | The contract has not been initialized yet. |
| `2` | `AlreadyInitialized` | The contract is already initialized; double-initialization is rejected. |
| `3` | `NoPendingTransfer` | No admin transfer is pending. |
| `4` | `ContractPaused` | The contract is paused; state-changing calls are rejected until an admin calls `unpause`. |
| `5` | `InvalidRegistryWiring` | The configured attester-registry address is invalid or unreachable. |
| `6` | `InvalidToken` | The configured token address is invalid or unreachable. |
| `7` | `InsufficientPoolBalance` | The pool does not hold enough tokens to cover the requested payout. |
| `8` | `WorkItemAlreadyApproved` | This work item has already been approved. |
| `9` | `WorkItemNotApproved` | This work item has not been approved by the approver. |
| `10` | `WorkItemAlreadyClaimed` | This work item has already been claimed (replay protection). |
| `11` | `AttesterNotAllowlisted` | The claiming attester is not currently allowlisted. |
| `12` | `AttesterClaimCapExceeded` | The payout would exceed the per-attester cumulative claim cap. |
| `13` | `PayoutCapExceeded` | The payout would exceed the per-claim cap. |
| `14` | `TransferFailed` | The token transfer call returned an error. |
| `15` | `NonPositiveAmount` | A non-positive amount was provided where a positive amount is required. |
