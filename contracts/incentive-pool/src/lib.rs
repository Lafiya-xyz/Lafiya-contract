I'll implement the multi-asset refactor along with the related features. Given the scope, I'll produce a complete, coherent file that compiles against the existing test modules' expectations as closely as possible while implementing the requested changes.

```rust
#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
#![allow(missing_docs)] // soroban macros generate undocumented public items

//! # Incentive Pool
//!
//! Bounded multi-asset payout/escrow contract for the M2 milestone.
//! Donors fund the pool, an approver authorizes work items, and
//! approved allowlisted attesters claim per-item payouts with
//! per-attestation replay protection.
//!
//! Accounting is keyed per allowlisted asset: deposits, payouts,
//! reservations, caps and drift are all tracked independently per asset.
//! There is no on-chain conversion between assets.

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, token,
    xdr::ToXdr, Address, Bytes, BytesN, Env, Map, Symbol, Vec,
};

/// The subset of the `attester-registry` contract this crate calls.
/// Kept as a trait interface (rather than a direct crate dependency)
/// so that `attester-registry`'s own contract implementation never
/// links into this crate's wasm — only the typed cross-contract
/// call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
    fn is_payout_eligible(env: Env, attester: Address) -> bool;
    /// Stable identity key for an attester, derived from the canonical
    /// licence hash and following rotations. Optional: older registries
    /// do not implement it, in which case the pool falls back to
    /// per-address caps.
    fn get_identity_key(env: Env, attester: Address) -> BytesN<32>;
    /// Interface negotiation. Returns a bitmask of supported features.
    fn get_interface(env: Env) -> u32;
}

/// Feature bit reported by `attester-registry::get_interface` when the
/// registry supports stable identity keys.
pub const REGISTRY_FEATURE_IDENTITY_KEY: u32 = 1 << 0;

/// Per-attester verification status as reported by `attestation-registry`.
/// Mirrors the registry's `AttesterAttestationStatus`; variant names must
/// stay identical because unit enum variants serialize by name.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttesterAttestationStatus {
    NeverAttested,
    Active,
    Withdrawn,
    Revoked,
}

/// The subset of the `attestation-registry` contract this crate calls to
/// prove a work item is backed by a live attestation. See ADR 0002.
#[contractclient(name = "AttestationRegistryClient")]
pub trait AttestationRegistryInterface {
    fn get_attester_attestation_status(
        env: Env,
        record_hash: BytesN<32>,
        attester: Address,
    ) -> AttesterAttestationStatus;
}

const SCHEMA_VERSION: u32 = 2;

/// Domain separator for work-item id derivation (see `derive_work_item_id`).
const WORK_ITEM_DOMAIN: &[u8] = b"LAFIYA-WORK-ITEM-v1";

/// Maximum number of items accepted by `approve_work_items`.
///
/// Each item writes one persistent `WorkItem` entry; the batch additionally
/// writes the instance entry and one epoch counter (18 writes at cap). Each
/// item typically reads 4 entries (84 reads at cap with distinct attesters),
/// under the per-transaction read/write entry limits. See
/// `budgets/incentive-pool-batch-approval.json` for the footprint analysis.
pub const MAX_BATCH_APPROVALS: u32 = 16;

/// Maximum number of items accepted by `disburse` (push mode).
///
/// Derived from budget measurements: each item performs one token transfer
/// plus a handful of storage reads/writes. 8 keeps the per-call footprint
/// comfortably under the per-transaction limits even with distinct
/// recipients. See `budgets/incentive-pool-disburse.json`.
pub const MAX_BATCH_DISBURSE: u32 = 8;

/// Base for the index-bearing error returned by `approve_work_items`:
/// `BATCH_ERROR_BASE + index * 100 + inner_error_code`.
pub const BATCH_ERROR_BASE: u32 = 10_000;

/// Delay before a newly designated payout address takes effect (48 hours).
/// Gives an attester time to notice and suspend a takeover of their key.
pub const PAYOUT_ADDRESS_DELAY: u64 = 48 * 60 * 60;

/// Delay before a proposed epoch-budget change can be applied (48 hours).
pub const EPOCH_CONFIG_DELAY: u64 = 48 * 60 * 60;

/// Delay before a proposed payout-schedule change can be applied (48 hours).
pub const SCHEDULE_TIMELOCK_SECS: u64 = 48 * 60 * 60;

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Default approval lifetime: 90 days, in seconds of ledger time.
/// Also applied to work items approved before expiry existed (see ADR 0018).
pub const DEFAULT_APPROVAL_TTL_SECS: u64 = 90 * 24 * 60 * 60;
/// Default upper bound on any approval lifetime: 365 days.
pub const DEFAULT_MAX_APPROVAL_TTL_SECS: u64 = 365 * 24 * 60 * 60;

/// Storage keys for the incentive pool.
///
/// UPGRADE SAFETY: `#[contracttype]` enums serialize variants by their
/// position index, so variant order and existing variants must never change
/// — append new variants at the end only. Reordering breaks decoding of
/// data written by earlier versions.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The address authorized to manage pool funds, set configuration,
    /// and upgrade the contract.
    Admin,
    /// Pending admin address for two-step admin transfer.
    PendingAdmin,
    /// The address authorized to approve work items for payout.
    Approver,
    /// Whether state-changing operations are currently paused.
    Paused,
    /// The storage schema version of the contract.
    SchemaVersion,
    /// The legacy single token contract address held by this pool.
    /// Retained for migration; new accounting is keyed per asset.
    Token,
    /// The deployed `attester-registry` contract consulted on every `claim`.
    AttesterRegistry,
    /// Legacy cumulative amount deposited into the pool (i128).
    /// Retained for migration only.
    TotalDeposited,
    /// Legacy cumulative amount paid out from the pool (i128).
    /// Retained for migration only.
    TotalPaid,
    /// Per-claim payout cap. Default: i128::MAX (uncapped).
    MaxPerClaim,
    /// Per-attester cumulative claim cap. Default: i128::MAX (uncapped).
    MaxPerAttester,
    /// Work-item info: stores the approved attester, payout amount,
    /// and approval timestamp. Absent key means the item is not approved.
    WorkItem(BytesN<32>),
    /// Whether a work item has been claimed. Absent means not yet claimed.
    WorkItemClaimed(BytesN<32>),
    /// Cumulative amount paid to a given attester across all claims.
    AttesterTotalClaimed(Address),
    /// The deployed `attestation-registry` consulted on every approval.
    AttestationRegistry,
    /// Payout recipient binding for an attester (`PayoutBinding`).
    PayoutBinding(Address),
    /// Active epoch budget configuration (`EpochConfig`).
    EpochConfig,
    /// Proposed epoch budget configuration awaiting its delay.
    PendingEpochConfig,
    /// Amount approved in the epoch starting at the given timestamp.
    EpochApproved(u64),
    // ── Appended variants (schema v2) ──
    /// Whether a balance mismatch is blocking `claim`.
    DriftDetected,
    /// Default approval TTL in seconds.
    DefaultApprovalTtl,
    /// Maximum approval TTL in seconds.
    MaxApprovalTtl,
    /// Current payout-schedule version.
    ScheduleVersion,
    /// Active payout rate for (`work_type`, `region_class`).
    PayoutRate(u32, u32),
    /// Pending (timelocked) payout rate for (`work_type`, `region_class`).
    PendingPayoutRate(u32, u32),
    /// Schedule version used to price a work item.
    WorkItemScheduleVersion(BytesN<32>),
    /// V2 work item record (with expiry).
    WorkItemV2(BytesN<32>),
    /// Cancelled work-item tombstone.
    WorkItemCancelled(BytesN<32>),
    /// Swept work-item marker.
    WorkItemSwept(BytesN<32>),
    /// Total reserved (approved but unclaimed) amount.
    TotalReserved,
    /// Proposal TTL in seconds.
    ProposalTtl,
    /// Per-asset cumulative amount deposited (i128).
    TotalDepositedFor(Address),
    /// Per-asset cumulative amount paid out (i128).
    TotalPaidFor(Address),
    /// Per-asset total reserved (approved but unclaimed) amount.
    TotalReservedFor(Address),
    /// Per-asset drift-blocked flag.
    DriftDetectedFor(Address),
    /// Per-asset per-claim cap.
    MaxPerClaimFor(Address),
    /// Per-asset per-attester cap.
    MaxPerAttesterFor(Address),
    /// Per-asset per-identity cap.
    MaxPerIdentityFor(Address),
    /// Whether an asset is allowlisted.
    AssetAllowlisted(Address),
    /// Metadata for an allowlisted asset.
    AssetInfo(Address),
    /// Cumulative amount paid to an attester for a specific asset.
    AttesterTotalClaimedFor(Address, Address),
    /// Cumulative amount paid to an identity for a specific asset.
    IdentityClaimed(Address, BytesN<32>),
    /// Approver revocation watermark: items approved by `approver` at or
    /// after this timestamp are void.
    ApproverRevokedSince(Address),
    /// Guardian address authorized to void approvals.
    Guardian,
}

/// One entry of an `approve_work_items` batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalInput {
    pub record_hash: BytesN<32>,
    pub attester: Address,
    pub payout_amount: i128,
    /// Asset the work item is paid in.
    pub asset: Address,
}

/// Payout recipient for an attester. `pending` replaces `current` once
/// `effective_at` has passed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutBinding {
    pub current: Option<Address>,
    pub pending: Option<Address>,
    pub effective_at: u64,
    /// Whether the attester opted in to approver-initiated push
    /// disbursement to the bound payout address.
    pub push_enabled: bool,
}

/// Period budget bounding total approvals per epoch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochConfig {
    /// Epoch length in seconds. Epochs start at multiples of this value.
    pub epoch_length: u64,
    /// Maximum total `payout_amount` approved within one epoch.
    pub epoch_budget: i128,
    /// When true, the unused budget of the immediately preceding epoch is
    /// added to the current one (one epoch of carry, never compounding).
    pub rollover: bool,
}

/// An epoch config change waiting for `EPOCH_CONFIG_DELAY`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEpochConfig {
    pub config: EpochConfig,
    pub effective_at: u64,
}

/// On-chain record of an approved work item.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItem {
    /// The attester authorized to claim payout for this item.
    pub attester: Address,
    /// The payout amount for this item.
    pub payout_amount: i128,
    /// Ledger timestamp when the item was approved.
    pub approved_at: u64,
    /// Ledger timestamp after which the item can no longer be claimed.
    /// The item is claimable while `now < expires_at`.
    pub expires_at: u64,
    /// The asset this item is paid in.
    pub asset: Address,
    /// The approver that approved this item.
    pub approved_by: Address,
}

/// Pre-expiry work-item shape, still stored under `DataKey::WorkItem` for
/// items approved before the upgrade. Read-only.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyWorkItem {
    attester: Address,
    payout_amount: i128,
    approved_at: u64,
}

/// Result of `reconcile`: internal accounting versus custody for one asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reconciliation {
    /// `total_deposited - total_paid`: what the pool believes it holds.
    pub accounted: i128,
    /// The pool's actual token balance.
    pub actual_balance: i128,
    /// `actual_balance - accounted`. Negative means custody shortfall.
    pub drift: i128,
}

/// Pending M-of-N approval of a work item.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub attester: Address,
    pub payout_amount: i128,
    pub programme: Option<Symbol>,
    pub endorsements: Vec<Address>,
    pub expires_at: u64,
}

/// Per-approver rate limit applied to every approval or endorsement.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimit {
    pub window_seconds: u64,
    pub max_count: u32,
    pub max_value: i128,
}

/// One compact counter per approver; reset when the window rolls over,
/// so storage never grows beyond one entry per approver.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproverWindow {
    pub window_start: u64,
    pub count: u32,
    pub value: i128,
}

/// Funding totals exposed for reconciliation. Invariant:
/// `custody == programmes_total + unallocated`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingTotals {
    pub deposited: i128,
    pub paid: i128,
    pub custody: i128,
    pub programmes_total: i128,
    pub unallocated: i128,
}

/// Metadata recorded for an allowlisted asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetInfo {
    /// Number of decimals reported by the token (best-effort).
    pub decimals: u32,
    /// Whether the token looks like a Stellar Asset Contract.
    pub is_sac: bool,
}

/// Active payout-schedule entry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutRate {
    pub amount: i128,
    pub version: u32,
    /// Asset the rate is denominated in.
    pub asset: Address,
}

/// Pending (timelocked) payout-schedule entry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingPayoutRate {
    pub amount: i128,
    pub eta: u64,
    /// Asset the rate is denominated in.
    pub asset: Address,
}

// ──────────────────────────── Events ────────────────────────────

#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminTransferred {
    #[topic]
    pub previous_admin: Address,
    #[topic]
    pub new_admin: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PoolFunded {
    #[topic]
    pub funder: Address,
    #[topic]
    pub asset: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PoolWithdrawn {
    #[topic]
    pub to: Address,
    #[topic]
    pub asset: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct WorkItemApproved {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub asset: Address,
    pub attester: Address,
    pub payout_amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PayoutRateProposed {
    #[topic]
    pub work_type: u32,
    #[topic]
    pub region_class: u32,
    #[topic]
    pub asset: Address,
    pub amount: i128,
    pub eta: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PayoutRateApplied {
    #[topic]
    pub work_type: u32,
    #[topic]
    pub region_class: u32,
    #[topic]
    pub asset: Address,
    pub amount: i128,
    pub version: u32,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PayoutClaimed {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub attester: Address,
    #[topic]
    pub asset: Address,
    pub recipient: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PayoutDisbursed {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub attester: Address,
    #[topic]
    pub asset: Address,
    pub recipient: Address,
    pub amount: i128,
    pub by: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PayoutAddressScheduled {
    #[topic]
    pub attester: Address,
    pub recipient: Address,
    pub effective_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PushModeChanged {
    #[topic]
    pub attester: Address,
    pub enabled: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttestationRegistryRepointed {
    #[topic]
    pub previous: Option<Address>,
    #[topic]
    pub new: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct EpochConfigProposed {
    pub epoch_length: u64,
    pub epoch_budget: i128,
    pub rollover: bool,
    pub effective_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct EpochConfigApplied {
    pub epoch_length: u64,
    pub epoch_budget: i128,
    pub rollover: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterRegistryRepointed {
    #[topic]
    pub previous: Address,
    #[topic]
    pub new: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ApproverSetChanged {
    #[topic]
    pub by: Address,
    pub approvers: Vec<Address>,
    pub threshold: u32,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct WorkItemProposed {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub proposer: Address,
    pub attester: Address,
    pub payout_amount: i128,
    pub expires_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct WorkItemEndorsed {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub endorser: Address,
    pub endorsements: u32,
    pub threshold: u32,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct RateLimitSet {
    #[topic]
    pub by: Address,
    pub window_seconds: u64,
    pub max_count: u32,
    pub max_value: i128,
}

/// Published on the approval that fills an approver's window; later
/// attempts in the same window fail with `ApproverRateLimited`.
#[contractevent]
#[derive(Clone, Debug)]
pub struct ApproverRateLimitHit {
    #[topic]
    pub approver: Address,
    pub count: u32,
    pub value: i128,
    pub window_resets_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProgrammeFunded {
    #[topic]
    pub donor: Address,
    #[topic]
    pub programme: Symbol,
    pub amount: i128,
}

/// Links an approved work item to the programme it draws from, so
/// indexers can attribute `payout_claimed` amounts to programmes.
#[contractevent]
#[derive(Clone, Debug)]
pub struct WorkItemProgrammeLinked {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub programme: Symbol,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ProgrammeRefunded {
    #[topic]
    pub donor: Address,
    #[topic]
    pub programme: Symbol,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Paused {
    #[topic]
    pub by: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Unpaused {
    #[topic]
    pub by: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct BalanceMismatch {
    pub expected: i128,
    pub observed: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct DriftResolved {
    #[topic]
    pub by: Address,
    pub drift: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AssetAdded {
    #[topic]
    pub asset: Address,
    pub decimals: u32,
    pub is_sac: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AssetRemoved {
    #[topic]
    pub asset: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ApprovalsVoided {
    #[topic]
    pub approver: Address,
    #[topic]
    pub by: Address,
    pub since: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct ReservationSwept {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub asset: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct Migrated {
    #[topic]
    pub asset: Address,
    pub deposited: i128,
    pub paid: i128,
}

// ──────────────────────────── Errors ────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// The contract is already initialized; double-initialization is rejected.
    AlreadyInitialized = 2,
    /// No admin transfer is pending.
    NoPendingTransfer = 3,
    /// The contract is paused; state-changing calls are rejected until
    /// an admin calls `unpause`.
    ContractPaused = 4,
    /// The configured attester-registry address is invalid or unreachable.
    InvalidRegistryWiring = 5,
    /// The configured token address is invalid or unreachable.
    InvalidToken = 6,
    /// The pool does not hold enough tokens to cover the requested payout.
    InsufficientPoolBalance = 7,
    /// This work item has already been approved.
    WorkItemAlreadyApproved = 8,
    /// This work item has not been approved by the approver.
    WorkItemNotApproved = 9,
    /// This work item has already been claimed (replay protection).
    WorkItemAlreadyClaimed = 10,
    /// The claiming attester is not currently allowlisted.
    AttesterNotAllowlisted = 11,
    /// The payout would exceed the per-attester cumulative claim cap.
    AttesterClaimCapExceeded = 12,
    /// The payout would exceed the per-claim cap.
    PayoutCapExceeded = 13,
    /// The token transfer call returned an error.
    TransferFailed = 14,
    /// A non-positive amount was provided where a positive amount is required.
    NonPositiveAmount = 15,
    /// No attestation-registry has been configured.
    AttestationRegistryNotSet = 16,
    /// The attester has no active (unrevoked, unwithdrawn) attestation for
    /// the record hash.
    AttestationNotActive = 17,
    /// The approval would exceed the current epoch budget.
    EpochBudgetExceeded = 18,
    /// The batch is empty or larger than `MAX_BATCH_APPROVALS`.
    BatchSizeInvalid = 19,
    /// The same work item appears twice in one batch.
    DuplicateWorkItemInBatch = 20,
    /// No pending epoch configuration exists.
    NoPendingEpochConfig = 21,
    /// The admin delay for the pending change has not elapsed.
    DelayNotElapsed = 22,
    /// Epoch length or budget is invalid (zero / negative).
    InvalidEpochConfig = 23,
    /// Arithmetic overflow.
    ArithmeticOverflow = 24,
    /// The observed token balance delta did not match the requested amount.
    TokenBalanceMismatch = 25,
    /// The attester is not currently eligible for payouts (probation).
    PayoutNotEligible = 26,
    /// Accounting and custody disagree; admin must call `resolve_drift`.
    ReconciliationRequired = 27,
    /// The work item has expired.
    WorkItemExpired = 28,
    /// The work item has been cancelled.
    WorkItemCancelled = 29,
    /// The work item has not yet expired.
    WorkItemNotExpired = 30,
    /// The approval TTL configuration is invalid.
    InvalidApprovalTtl = 31,
    /// No pending payout-schedule change exists.
    NoPendingScheduleChange = 32,
    /// The payout-schedule timelock has not elapsed.
    TimelockNotElapsed = 33,
    /// No payout-schedule entry exists for the pair.
    ScheduleEntryNotFound = 34,
    /// The asset is not allowlisted.
    AssetNotAllowlisted = 35,
    /// The asset is already allowlisted.
    AssetAlreadyAllowlisted = 36,
    /// The asset failed the interface probe.
    InvalidAsset = 37,
    /// The approver has been revoked for this item's approval time.
    ApprovalVoided = 38,
    /// Push disbursement is not enabled for this attester.
    PushNotEnabled = 39,
    /// The disburse batch is empty or too large.
    DisburseBatchInvalid = 40,
    /// The payout would exceed the per-identity cumulative claim cap.
    IdentityClaimCapExceeded = 41,
    /// The migration has already been performed.
    AlreadyMigrated = 42,
    /// The caller is not authorized for this operation.
    Unauthorized = 43,
}

// ──────────────────────────── Contract ────────────────────────────

/// Version of the public contract interface (functions, errors, types).
pub const INTERFACE_VERSION: u32 = 2;

// Source-provenance metadata (SEP-46 `contractmetav0`); policy in
// docs/stability-policy.md, enforced by scripts/check_wasm_meta.py.
soroban_sdk::contractmeta!(
    key = "source_repo",
    val = "github:Lafiya-xyz/Lafiya-contract"
);
soroban_sdk::contractmeta!(key = "home_domain", val = "lafiya-xyz.github.io");
soroban_sdk::contractmeta!(key = "crate_name", val = env!("CARGO_PKG_NAME"));
soroban_sdk::contractmeta!(key = "crate_version", val = env!("CARGO_PKG_VERSION"));
soroban_sdk::contractmeta!(key = "source_rev", val = env!("LAFIYA_GIT_COMMIT"));
soroban_sdk::contractmeta!(key = "interface_version", val = "2");

#[contract]
pub struct IncentivePool;

#[contractimpl]
impl IncentivePool {
    /// Initialize the pool. Sets the admin, approver, token contract,
    /// attester-registry contract, and default caps. Can only be called once.
    ///
    /// The caller must authorize as `admin`. A best-effort interface check
    /// is performed against both `attester_registry` and `token`. The
    /// initial `token` is added to the asset allowlist.
    pub fn initialize(
        env: Env,
        admin: Address,
        approver: Address,
        token: Address,
        attester_registry: Address,
        max_per_claim: i128,
        max_per_attester: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        // Best-effort sanity check: verify attester_registry implements
        // the is_attester interface.
        let registry = AttesterRegistryClient::new(&env, &attester_registry);
        let throwaway = env.current_contract_address();
        if registry.try_is_attester(&throwaway).is_err() {
            return Err(Error::InvalidRegistryWiring);
        }

        // Best-effort sanity check: verify token implements the token interface
        // by calling balance with the current contract address.
        let token_client = token::Client::new(&env, &token);
        let self_addr = env.current_contract_address();
        if token_client.try_balance(&self_addr).is_err() {
            return Err(Error::InvalidToken);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Approver, &approver);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &attester_registry);
        env.storage()
            .instance()
            .set(&DataKey::MaxPerClaim, &max_per_claim);
        env.storage()
            .instance()
            .set(&DataKey::MaxPerAttester, &max_per_attester);
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &0_i128);
        env.storage().instance().set(&DataKey::TotalPaid, &0_i128);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);

        // Allowlist the initial token and seed per-asset caps.
        let info = Self::probe_asset(&env, &token)?;
        env.storage()
            .persistent()
            .set(&DataKey::AssetAllowlisted(token.clone()), &true);
        env.storage()
            .persistent()
            .set(&DataKey::AssetInfo(token.clone()), &info);
        env.storage()
            .instance()
            .set(&DataKey::MaxPerClaimFor(token.clone()), &max_per_claim);
        env.storage()
            .instance()
            .set(&DataKey::MaxPerAttesterFor(token.clone()), &max_per_attester);
        env.storage()
            .instance()
            .set(&DataKey::TotalDepositedFor(token.clone()), &0_i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalPaidFor(token.clone()), &0_i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalReservedFor(token.clone()), &0_i128);

        AssetAdded {
            asset: token,
            decimals: info.decimals,
            is_sac: info.is_sac,
        }
        .publish(&env);

        Initialized { admin }.publish(&env);
        Ok(())
    }

    // ── Configuration getters ──

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Return the current approver address.
    pub fn get_approver(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Approver)
            .ok_or(Error::NotInitialized)
    }

    /// Return the legacy single token contract address (migration only).
    pub fn get_token(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(Error::NotInitialized)
    }

    /// Return the configured attester-registry contract address.
    pub fn get_attester_registry(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)
    }

    /// Return the per-claim payout cap for `asset`.
    pub fn get_max_per_claim(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::MaxPerClaimFor(asset))
            .unwrap_or(i128::MAX))
    }

    /// Return the per-attester cumulative claim cap for `asset`.
    pub fn get_max_per_attester(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::MaxPerAttesterFor(asset))
            .unwrap_or(i128::MAX))
    }

    /// Return the per-identity cumulative claim cap for `asset`.
    pub fn get_max_per_identity(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::MaxPerIdentityFor(asset))
            .unwrap_or(i128::MAX))
    }

    /// Return the cumulative amount deposited into the pool for `asset`.
    pub fn get_total_deposited(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(Self::total_deposited(&env, &asset))
    }

    /// Return the cumulative amount paid out from the pool for `asset`.
    pub fn get_total_paid(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(Self::total_paid(&env, &asset))
    }

    /// Return the total reserved (approved but unclaimed) amount for `asset`.
    pub fn get_total_reserved(env: Env, asset: Address) -> Result<i128, Error> {
        Ok(Self::total_reserved(&env, &asset))
    }

    /// Whether a work item has been approved.
    pub fn is_work_item_approved(env: Env, work_item_id: BytesN<32>) -> bool {
        Self::has_work_item(&env, &work_item_id)
    }

    /// Whether a work item has been claimed.
    pub fn is_work_item_claimed(env: Env, work_item_id: BytesN<32>) -> bool {
        Self::is_claimed(&env, &work_item_id)
    }

    /// Return the work item details, if approved. Legacy items report
    /// `expires_at = approved_at + DEFAULT_APPROVAL_TTL_SECS`.
    pub fn get_work_item(env: Env, work_item_id: BytesN<32>) -> Option<WorkItem> {
        Self::load_work_item(&env, &work_item_id).ok()
    }

    /// Return the cumulative amount claimed by an attester for `asset`.
    pub fn get_attester_total_claimed(env: Env, attester: Address, asset: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::AttesterTotalClaimedFor(attester, asset))
            .unwrap_or(