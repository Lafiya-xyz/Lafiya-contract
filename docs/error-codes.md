# Lafiya Smart Contract Error Codes

This document enumerates the error codes defined in the Lafiya Soroban smart contracts.

> [!IMPORTANT]
> **Error codes are contract-scoped, not global.** Each contract defines its own `Error` enum starting from `1`. To correctly interpret an error code, you must know which contract produced the error.

For programmatic decoding, use the generated [`catalog/errors.json`](../catalog/errors.json) or `decodeContractError(contractKind, code)` from the bindings' `catalog` export instead of hard-coding this table.

## `attester-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | `initialize` has not been called yet; call `initialize(admin: Address)` before using the contract. |
| `2` | `AlreadyInitialized` | `initialize` was called more than once. |
| `3` | `NoPendingTransfer` | `accept_admin` was called with no pending admin transfer. Admin transfer is a two-step flow: the current admin must first call `propose_admin` to nominate a successor, then the nominated address must call `accept_admin` to complete the transfer. This error is returned when `accept_admin` is called before a corresponding `propose_admin` call has set a pending admin. |
| `4` | `ContractPaused` | The requested operation is blocked while the contract is paused. |
| `5` | `AllowlistFull` | The allowlist is at its configured maximum size. Raise the cap via `set_max_attesters`, or free a slot via `remove_attester`. |
| `6` | `MigrationNotRequired` | `migrate()` was called while the stored schema version is already `>= SCHEMA_VERSION`. Only call `migrate()` after `upgrade()` to a build that bumps `SCHEMA_VERSION`; this error is a safe no-op signal that there is nothing pending, not a failure to react to. |
| `7` | `AttesterNotFound` | The referenced attester is not currently allowlisted (never added, or since removed). |
| `8` | `BatchTooLarge` | The supplied batch exceeds `BATCH_LIMIT` addresses. |
| `9` | `InvalidAdminProposal` | The proposed admin address is not a valid successor. |
| `10` | `ProposalExpired` | The pending admin proposal has expired. |
| `11` | `RoleNotGranted` | The supplied address has not been granted the required role. |
| `12` | `InvalidValidityWindow` | The attester validity window is empty or reversed. |
| `13` | `RegionMismatch` | The requested attester region is outside the registrar's assigned region. |
| `14` | `RegionalQuotaExceeded` | The registrar's concurrent enrollment quota has been reached. |
| `15` | `RegionalAttestersRemain` | The registrar's region cannot change while its attesters remain enrolled. |
| `16` | `InvalidRegion` | The region is not a valid ISO 3166-2 style code such as `NG-LA`. |

## `attestation-registry`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `NotInitialized` | Required registry configuration is missing from storage. |
| `2` | `AlreadyInitialized` | Reserved for compatibility with the removed public initializer. |
| `3` | `AttesterNotAllowlisted` | The caller is not allowlisted by the `attester-registry` contract. |
| `4` | `NoPendingTransfer` | `accept_admin` was called with no pending admin transfer. Admin transfer is a two-step flow: the current admin must first call `propose_admin` to nominate a successor, then the nominated address must call `accept_admin` to complete the transfer. This error is returned when `accept_admin` is called before a corresponding `propose_admin` call has set a pending admin. |
| `5` | `InvalidRegistryWiring` | The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration. |
| `6` | `AttestationNotFound` | No attestation exists for the given record hash / sequence. |
| `7` | `ContractPaused` | The requested operation is blocked while the contract is paused. |
| `8` | `InvalidAdminProposal` | The proposed admin address is not a valid successor. |
| `9` | `ProposalExpired` | The pending admin proposal has expired. |
| `10` | `RateLimited` | The attester has used up its rate-limit window. Call `get_rate_limit_retry_after` for the first ledger it may attest again. |
| `11` | `InvalidRateLimit` | `window_ledgers` was `0` or longer than 30 days of ledgers. |
| `12` | `RoleNotGranted` | The supplied address has not been granted the required role. |
| `13` | `MigrationNotRequired` | `migrate()` was called when no storage migration is pending. |
| `14` | `PatientConsentRequired` | No unexpired patient consent grant matches this attester, patient and record hash. |
| `15` | `PatientConsentExpired` | The patient's consent grant has expired. |
| `16` | `TimestampOverflow` | The current ledger timestamp cannot be safely advanced to calculate a time bound. |
| `17` | `InvalidAttestationExpiry` | The patient-selected attestation expiry is not in the future. |
| `18` | `AttesterRegistryUnavailable` | The configured attester-registry could not be called. |
| `19` | `EmptyBatch` | A Merkle batch must contain at least one record. |
| `20` | `BatchAlreadyAnchored` | The Merkle root has already been anchored. |
| `21` | `AttestationNotOwned` | The attester has no active attestation for the given record hash. |
| `22` | `InvalidRecordVersion` | The supplied previous hash or version relationship is invalid. |
| `23` | `BatchTooLarge` | The batch contains more requests than the supported maximum. |
| `24` | `InvalidCommitmentVersion` | The commitment scheme version does not fit in one byte. |

## `multisig-account`

| Error Code (u32) | Variant Name | Description |
|---|---|---|
| `1` | `InvalidThreshold` | The configured threshold is zero or exceeds the signer count. |
| `2` | `DuplicateSigner` | The signer configuration contains duplicate public keys. |
| `3` | `NotEnoughSigners` | The supplied signature count is below the configured threshold. |
| `4` | `BadSignatureOrder` | Signatures are not strictly ordered by key type and public-key bytes. |
| `5` | `UnknownSigner` | A signature corresponds to a public key that is not a configured signer. |
| `6` | `NotInitialized` | The contract has not been initialized; threshold or signer count is unavailable. |
| `7` | `TooManySigners` | The supplied signature count exceeds the configured signer count. |
| `8` | `NotEnoughWeight` | The supplied signatures do not meet the configured weight threshold. |
| `9` | `RoleQuorumNotMet` | The supplied signatures do not meet one or more role requirements. |
| `10` | `InvalidPolicy` | A signer weight, role requirement, or policy threshold is invalid. |

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
