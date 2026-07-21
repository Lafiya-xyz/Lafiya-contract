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

## `attestation-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | The contract has not been initialized yet. |
| `2` | `AlreadyInitialized` | The contract is already initialized; double-initialization is rejected. |
| `3` | `AttesterNotAllowlisted` | The attester address is not allowlisted in the configured `attester-registry` contract. |
| `4` | `NoPendingTransfer` | No admin transfer is pending. |
| `5` | `InvalidRegistryWiring` | The configured `attester-registry` contract address did not respond as expected to the `is_attester` cross-contract call. |
| `6` | `AttestationNotFound` | No attestation exists for the given `record_hash`. |
| `7` | `ContractPaused` | The contract is paused; `attest` is rejected until an admin calls `unpause`. |
