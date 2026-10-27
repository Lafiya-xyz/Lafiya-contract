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
//! Bounded USDC payout/escrow contract for the M2 milestone.
//! Donors fund the pool, an approver authorizes work items, and
//! approved allowlisted attesters claim per-item payouts with
//! per-attestation replay protection.

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, token,
    xdr::ToXdr, Address, Bytes, BytesN, Env, Map, Vec,
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
}

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

const SCHEMA_VERSION: u32 = 1;

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

/// Base for the index-bearing error returned by `approve_work_items`:
/// `BATCH_ERROR_BASE + index * 100 + inner_error_code`.
pub const BATCH_ERROR_BASE: u32 = 10_000;

/// Delay before a newly designated payout address takes effect (48 hours).
/// Gives an attester time to notice and suspend a takeover of their key.
pub const PAYOUT_ADDRESS_DELAY: u64 = 48 * 60 * 60;

/// Delay before a proposed epoch-budget change can be applied (48 hours).
pub const EPOCH_CONFIG_DELAY: u64 = 48 * 60 * 60;

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
    /// The USDC (or other soroban-token) contract address held by this pool.
    Token,
    /// The deployed `attester-registry` contract consulted on every `claim`.
    AttesterRegistry,
    /// Cumulative amount deposited into the pool (i128).
    TotalDeposited,
    /// Cumulative amount paid out from the pool (i128).
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
}

/// One entry of an `approve_work_items` batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalInput {
    pub record_hash: BytesN<32>,
    pub attester: Address,
    pub payout_amount: i128,
}

/// Payout recipient for an attester. `pending` replaces `current` once
/// `effective_at` has passed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutBinding {
    pub current: Option<Address>,
    pub pending: Option<Address>,
    pub effective_at: u64,
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

/// Result of `reconcile`: internal accounting versus custody.
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
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PoolWithdrawn {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct WorkItemApproved {
    #[topic]
    pub work_item_id: BytesN<32>,
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
    pub recipient: Address,
    pub amount: i128,
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
}

// ──────────────────────────── Contract ────────────────────────────

/// Version of the public contract interface (functions, errors, types).
pub const INTERFACE_VERSION: u32 = 1;

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
soroban_sdk::contractmeta!(key = "interface_version", val = "1");

#[contract]
pub struct IncentivePool;

#[contractimpl]
impl IncentivePool {
    /// Initialize the pool. Sets the admin, approver, token contract,
    /// attester-registry contract, and default caps. Can only be called once.
    ///
    /// The caller must authorize as `admin`. A best-effort interface check
    /// is performed against both `attester_registry` and `token`.
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

    /// Return the configured token contract address.
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

    /// Return the per-claim payout cap.
    pub fn get_max_per_claim(env: Env) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::MaxPerClaim)
            .unwrap_or(i128::MAX))
    }

    /// Return the per-attester cumulative claim cap.
    pub fn get_max_per_attester(env: Env) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::MaxPerAttester)
            .unwrap_or(i128::MAX))
    }

    /// Return the cumulative amount deposited into the pool.
    pub fn get_total_deposited(env: Env) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0))
    }

    /// Return the cumulative amount paid out from the pool.
    pub fn get_total_paid(env: Env) -> Result<i128, Error> {
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalPaid)
            .unwrap_or(0))
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

    /// Return the cumulative amount claimed by an attester.
    pub fn get_attester_total_claimed(env: Env, attester: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::AttesterTotalClaimed(attester))
            .unwrap_or(0)
    }

    // ── Admin: two-step transfer ──

    /// Propose a new admin address. Requires admin auth.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::admin(&env)?;
        current_admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        Ok(())
    }

    /// Accept the proposed admin transfer. Requires pending-admin auth.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let previous_admin = Self::admin(&env)?;
        let pending_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingTransfer)?;

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);

        AdminTransferred {
            previous_admin,
            new_admin: pending_admin,
        }
        .publish(&env);

        Ok(())
    }

    // ── Admin: approver management ──

    /// Set the approver address authorized to approve work items.
    /// Requires admin auth.
    pub fn set_approver(env: Env, new_approver: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Approver, &new_approver);
        Ok(())
    }

    // ── Admin: registry repointing ──

    /// Change the attester-registry contract. Requires admin auth.
    pub fn set_attester_registry(env: Env, new_registry: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let previous = Self::attester_registry(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::AttesterRegistry, &new_registry);

        AttesterRegistryRepointed {
            previous,
            new: new_registry,
        }
        .publish(&env);

        Ok(())
    }

    // ── Admin: cap configuration ──

    /// Set the per-claim payout cap. Requires admin auth.
    pub fn set_max_per_claim(env: Env, max: i128) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage().instance().set(&DataKey::MaxPerClaim, &max);
        Ok(())
    }

    /// Set the per-attester cumulative claim cap. Requires admin auth.
    pub fn set_max_per_attester(env: Env, max: i128) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        env.storage().instance().set(&DataKey::MaxPerAttester, &max);
        Ok(())
    }

    // ── Admin: fund / withdraw ──

    /// Deposit `amount` tokens from the caller into the pool.
    /// Requires admin auth. The admin must have previously approved
    /// the pool contract to spend their tokens, or the transaction
    /// must include the inner `token.transfer` in the authorization tree.
    ///
    /// Only the observed change in the pool's token balance is credited,
    /// so fee-on-transfer tokens cannot inflate `TotalDeposited`. If the
    /// observed delta differs from `amount`, a `BalanceMismatch` event is
    /// emitted and `claim` is blocked until the admin calls `resolve_drift`.
    pub fn fund(env: Env, amount: i128) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        Self::require_not_paused(&env)?;

        if amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }

        let token_addr = Self::token(&env)?;
        let pool_addr = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        let before = token_client.balance(&pool_addr);
        token_client.transfer(&admin, &pool_addr, &amount);
        let after = token_client.balance(&pool_addr);
        let received = after.checked_sub(before).ok_or(Error::ArithmeticOverflow)?;
        if received <= 0 {
            return Err(Error::TokenBalanceMismatch);
        }
        if received != amount {
            env.storage().instance().set(&DataKey::DriftDetected, &true);
            BalanceMismatch {
                expected: amount,
                observed: received,
            }
            .publish(&env);
        }

        let deposited = Self::total_deposited(&env)
            .checked_add(received)
            .ok_or(Error::ArithmeticOverflow)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &deposited);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        PoolFunded {
            funder: admin,
            amount: received,
        }
        .publish(&env);

        Ok(())
    }

    /// Withdraw `amount` tokens from the pool to `to`. Requires admin auth.
    /// This is the recovery mechanism for excess or misallocated funds.
    pub fn withdraw(env: Env, to: Address, amount: i128) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }

        let total_paid = Self::total_paid(&env);
        let available = Self::total_deposited(&env)
            .checked_sub(total_paid)
            .ok_or(Error::ArithmeticOverflow)?;
        if amount > available {
            return Err(Error::InsufficientPoolBalance);
        }

        let token_addr = Self::token(&env)?;
        let pool_addr = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        token_client.transfer(&pool_addr, &to, &amount);

        // Withdrawn funds leave the pool, so count them against the
        // deposited balance; otherwise `claim` would believe they are still
        // available and fail at the token transfer.
        let new_paid = total_paid
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::TotalPaid, &new_paid);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        PoolWithdrawn { to, amount }.publish(&env);

        Ok(())
    }

    // ── Admin: reconciliation ──

    /// Compare internal accounting with the pool's actual token balance.
    pub fn reconcile(env: Env) -> Result<Reconciliation, Error> {
        Self::reconciliation(&env)
    }

    /// Whether a balance mismatch is blocking `claim`.
    pub fn is_drift_detected(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::DriftDetected)
            .unwrap_or(false)
    }

    /// Accept the observed token balance as the new accounting baseline and
    /// unblock `claim`. Positive drift (e.g. a direct transfer) is credited
    /// as a deposit; negative drift (e.g. clawback) is debited. Requires
    /// admin auth. Fails if the debit would make `total_paid` exceed
    /// `total_deposited`.
    pub fn resolve_drift(env: Env) -> Result<Reconciliation, Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let rec = Self::reconciliation(&env)?;
        let deposited = Self::total_deposited(&env)
            .checked_add(rec.drift)
            .ok_or(Error::ArithmeticOverflow)?;
        if deposited < Self::total_paid(&env) {
            return Err(Error::InsufficientPoolBalance);
        }
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &deposited);
        env.storage()
            .instance()
            .set(&DataKey::DriftDetected, &false);

        DriftResolved {
            by: admin,
            drift: rec.drift,
        }
        .publish(&env);

        Ok(rec)
    }

    // ── Admin: approval TTL configuration ──

    /// Return `(default_ttl_secs, max_ttl_secs)` for work-item approvals.
    pub fn get_approval_ttl(env: Env) -> (u64, u64) {
        (Self::default_ttl(&env), Self::max_ttl(&env))
    }

    /// Set the default and maximum approval TTL in seconds. Requires admin
    /// auth. Both must be positive and `default_ttl <= max_ttl`. Applies to
    /// new approvals only; existing items keep their `expires_at`.
    pub fn set_approval_ttl(env: Env, default_ttl: u64, max_ttl: u64) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if default_ttl == 0 || default_ttl > max_ttl {
            return Err(Error::InvalidApprovalTtl);
        }
        env.storage()
            .instance()
            .set(&DataKey::DefaultApprovalTtl, &default_ttl);
        env.storage()
            .instance()
            .set(&DataKey::MaxApprovalTtl, &max_ttl);
        Ok(())
    }

    // ── Approver: approve work items ──

    /// Derive the work-item id for an attestation:
    /// `sha256("LAFIYA-WORK-ITEM-v1" || record_hash || attester.to_xdr())`.
    /// One attestation (record hash + attester) maps to exactly one work
    /// item, so an attestation can back at most one payout.
    pub fn derive_work_item_id(env: Env, record_hash: BytesN<32>, attester: Address) -> BytesN<32> {
        let mut preimage = Bytes::from_slice(&env, WORK_ITEM_DOMAIN);
        preimage.append(&Bytes::from(record_hash));
        preimage.append(&attester.to_xdr(&env));
        env.crypto().sha256(&preimage).into()
    }

    /// Approve the work item backed by `attester`'s attestation of
    /// `record_hash`. Requires the approver's auth. The attestation must be
    /// active in the attestation-registry, the attester allowlisted, the
    /// amount positive, within the per-claim cap and the epoch budget.
    /// Returns the derived work-item id.
    pub fn approve_work_item(
        env: Env,
        record_hash: BytesN<32>,
        attester: Address,
        payout_amount: i128,
    ) -> Result<BytesN<32>, Error> {
        Self::approver(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        let mut ctx = ApprovalCtx::load(&env)?;
        let id = Self::approve_one(
            &env,
            &mut ctx,
            ApprovalInput {
                record_hash,
                attester,
                payout_amount,
            },
        )?;
        ctx.commit(&env);
        Ok(id)
    }

    /// Approve up to `MAX_BATCH_APPROVALS` work items in one transaction.
    /// All-or-nothing: the first invalid entry aborts the whole batch with
    /// contract error `BATCH_ERROR_BASE + index * 100 + inner_code`.
    /// The approver is authorized once and each distinct attester is
    /// looked up in the attester-registry once. One `WorkItemApproved`
    /// event is emitted per item. Items are charged against the epoch
    /// budget cumulatively, in order.
    pub fn approve_work_items(
        env: Env,
        items: Vec<ApprovalInput>,
    ) -> Result<Vec<BytesN<32>>, soroban_sdk::Error> {
        Self::approver(&env)?.require_auth();
        Self::admin(&env)?.require_auth();
        Self::approve_internal(&env, work_item_id, attester, payout_amount, 0)
    }

    /// Approve a work item priced from the payout schedule entry for
    /// (`work_type`, `region_class`). Requires the approver's auth. The
    /// schedule version used is recorded against the work item.
    pub fn approve_scheduled_work_item(
        env: Env,
        work_item_id: BytesN<32>,
        attester: Address,
        work_type: u32,
        region_class: u32,
    ) -> Result<(), Error> {
        Self::approver(&env)?.require_auth();
        let rate = Self::get_payout_rate(env.clone(), work_type, region_class)
            .ok_or(Error::ScheduleEntryNotFound)?;
        Self::approve_internal(&env, work_item_id, attester, rate.amount, rate.version)
    }

    /// Schedule version used to price a work item (`0` = override path or
    /// approved before the schedule existed).
    pub fn get_work_item_schedule_version(env: Env, work_item_id: BytesN<32>) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::WorkItemScheduleVersion(work_item_id))
            .unwrap_or(0)
    }

    // ── Admin: payout schedule (timelocked) ──

    /// Propose a payout amount for (`work_type`, `region_class`). Takes
    /// effect only after `SCHEDULE_TIMELOCK_SECS` via `apply_payout_rate`.
    /// Requires admin auth. Emits `PayoutRateProposed`.
    pub fn propose_payout_rate(
        env: Env,
        work_type: u32,
        region_class: u32,
        amount: i128,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }
        let eta = env.ledger().timestamp() + SCHEDULE_TIMELOCK_SECS;
        env.storage().persistent().set(
            &DataKey::PendingPayoutRate(work_type, region_class),
            &PendingPayoutRate { amount, eta },
        );
        PayoutRateProposed {
            work_type,
            region_class,
            amount,
            eta,
        }
        .publish(&env);
        Ok(())
    }

    /// Apply a pending payout-schedule change once its timelock elapsed.
    /// Callable by anyone. Bumps the schedule version and emits
    /// `PayoutRateApplied`.
    pub fn apply_payout_rate(env: Env, work_type: u32, region_class: u32) -> Result<u32, Error> {
        let key = DataKey::PendingPayoutRate(work_type, region_class);
        let pending: PendingPayoutRate = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoPendingScheduleChange)?;
        if env.ledger().timestamp() < pending.eta {
            return Err(Error::TimelockNotElapsed);
        }
        let version = Self::get_schedule_version(env.clone()) + 1;
        env.storage()
            .instance()
            .set(&DataKey::ScheduleVersion, &version);
        env.storage().persistent().remove(&key);
        env.storage().persistent().set(
            &DataKey::PayoutRate(work_type, region_class),
            &PayoutRate {
                amount: pending.amount,
                version,
            },
        );
        PayoutRateApplied {
            work_type,
            region_class,
            amount: pending.amount,
            version,
        }
        .publish(&env);
        Ok(version)
    }

    /// Active schedule entry for (`work_type`, `region_class`), if any.
    pub fn get_payout_rate(env: Env, work_type: u32, region_class: u32) -> Option<PayoutRate> {
        env.storage()
            .persistent()
            .get(&DataKey::PayoutRate(work_type, region_class))
    }

    /// Pending (timelocked) schedule change for the pair, if any.
    pub fn get_pending_payout_rate(
        env: Env,
        work_type: u32,
        region_class: u32,
    ) -> Option<PendingPayoutRate> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingPayoutRate(work_type, region_class))
    }

    /// Current payout-schedule version (`0` before any change is applied).
    pub fn get_schedule_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ScheduleVersion)
            .unwrap_or(0)
    }

    fn approve_internal(
        env: &Env,
        work_item_id: BytesN<32>,
        attester: Address,
        payout_amount: i128,
        schedule_version: u32,
    ) -> Result<(), Error> {
        let env = env.clone();
        Self::require_not_paused(&env)?;

        if items.is_empty() || items.len() > MAX_BATCH_APPROVALS {
            return Err(Error::BatchSizeInvalid.into());
        }

        let mut ctx = ApprovalCtx::load(&env)?;
        let mut seen: Map<BytesN<32>, ()> = Map::new(&env);
        let mut ids = Vec::new(&env);
        for (index, item) in items.iter().enumerate() {
            let fail = |e: Error| {
                soroban_sdk::Error::from_contract_error(
                    BATCH_ERROR_BASE + (index as u32) * 100 + e as u32,
                )
            };
            let id = Self::derive_work_item_id(
                env.clone(),
                item.record_hash.clone(),
                item.attester.clone(),
            );
            if seen.contains_key(id.clone()) {
                return Err(fail(Error::DuplicateWorkItemInBatch));
            }
            seen.set(id, ());
            ids.push_back(Self::approve_one(&env, &mut ctx, item).map_err(fail)?);
        }
        ctx.commit(&env);
        Ok(ids)
    }

    fn approve_one(
        env: &Env,
        ctx: &mut ApprovalCtx,
        item: ApprovalInput,
    ) -> Result<BytesN<32>, Error> {
        let ApprovalInput {
            record_hash,
            attester,
            payout_amount,
        } = item;

        if payout_amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }
        if payout_amount > ctx.max_per_claim {
            return Err(Error::PayoutCapExceeded);
        }

        let work_item_id =
            Self::derive_work_item_id(env.clone(), record_hash.clone(), attester.clone());

        // Reject duplicate approval (and therefore attestation reuse).
        if env
            .storage()
            .persistent()
            .has(&DataKey::WorkItem(work_item_id.clone()))
        {
            return Err(Error::WorkItemAlreadyApproved);
        }
        .publish(&env);
        Ok(())
    }

        // Verify attester is currently allowlisted (memoized per call).
        let allowed = match ctx.allowlisted.get(attester.clone()) {
            Some(v) => v,
            None => {
                let v =
                    AttesterRegistryClient::new(env, &ctx.attester_registry).is_attester(&attester);
                ctx.allowlisted.set(attester.clone(), v);
                v
            }
        };
        if !allowed {
            return Err(Error::AttesterNotAllowlisted);
        }
        env.storage()
            .instance()
            .set(&DataKey::ProposalTtl, &seconds);
        Ok(())
    }

        // Proof of work: an active attestation by this attester.
        let status = AttestationRegistryClient::new(env, &ctx.attestation_registry)
            .get_attester_attestation_status(&record_hash, &attester);
        if status != AttesterAttestationStatus::Active {
            return Err(Error::AttestationNotActive);
        }

        // Epoch budget.
        if let Some(epoch) = ctx.epoch.as_mut() {
            if epoch.approved + payout_amount > epoch.available {
                return Err(Error::EpochBudgetExceeded);
            }
            epoch.approved += payout_amount;
        }
        .publish(&env);

        let work_item = WorkItem {
            attester: attester.clone(),
            payout_amount,
            programme,
            endorsements,
            expires_at,
        };
        env.storage()
            .persistent()
            .get(&DataKey::WorkItemCancelled(work_item_id))
    }

    /// Release the reservation of an expired, unclaimed work item.
    /// Permissionless. Idempotent: sweeping an already-swept item is a no-op
    /// returning `Ok` without an event. Never releases a claimed or
    /// cancelled item.
    pub fn sweep_expired(env: Env, work_item_id: BytesN<32>) -> Result<(), Error> {
        if Self::is_swept(&env, &work_item_id) {
            return Ok(());
        }
        let work_item = Self::load_work_item(&env, &work_item_id)?;
        if Self::is_claimed(&env, &work_item_id) {
            return Err(Error::WorkItemAlreadyClaimed);
        }
        if Self::is_cancelled(&env, &work_item_id) {
            return Err(Error::WorkItemCancelled);
        }
        if env.ledger().timestamp() < work_item.expires_at {
            return Err(Error::WorkItemNotExpired);
        }

        WorkItemApproved {
            work_item_id: work_item_id.clone(),
            attester,
            payout_amount,
        }
        .publish(env);

        Ok(work_item_id)
    }

    // ── Admin: attestation registry ──

    /// Set or repoint the attestation-registry used as proof of work.
    /// Requires admin auth; evented like `set_attester_registry`.
    pub fn set_attestation_registry(env: Env, new_registry: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if new_registry == env.current_contract_address() {
            return Err(Error::InvalidRegistryWiring);
        }
        let previous: Option<Address> = env.storage().instance().get(&DataKey::AttestationRegistry);
        env.storage()
            .instance()
            .set(&DataKey::AttestationRegistry, &new_registry);
        AttestationRegistryRepointed {
            previous,
            new: new_registry,
        }
        .publish(&env);
        Ok(())
    }

    pub fn get_attestation_registry(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::AttestationRegistry)
    }

    // ── Admin: epoch budgets ──

    /// Propose a new epoch budget. Takes effect via `apply_epoch_config`
    /// once `EPOCH_CONFIG_DELAY` has elapsed. Requires admin auth.
    pub fn propose_epoch_config(
        env: Env,
        epoch_length: u64,
        epoch_budget: i128,
        rollover: bool,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if epoch_length == 0 || epoch_budget < 0 {
            return Err(Error::InvalidEpochConfig);
        }
        let effective_at = env.ledger().timestamp() + EPOCH_CONFIG_DELAY;
        let pending = PendingEpochConfig {
            config: EpochConfig {
                epoch_length,
                epoch_budget,
                rollover,
            },
            effective_at,
        };
        env.storage()
            .instance()
            .set(&DataKey::PendingEpochConfig, &pending);
        EpochConfigProposed {
            epoch_length,
            epoch_budget,
            rollover,
            effective_at,
        }
        .publish(&env);
        Ok(())
    }

    /// Apply the pending epoch budget after its delay. Requires admin auth.
    pub fn apply_epoch_config(env: Env) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let pending: PendingEpochConfig = env
            .storage()
            .instance()
            .get(&DataKey::PendingEpochConfig)
            .ok_or(Error::NoPendingEpochConfig)?;
        if env.ledger().timestamp() < pending.effective_at {
            return Err(Error::DelayNotElapsed);
        }
        let c = pending.config;
        env.storage().instance().set(&DataKey::EpochConfig, &c);
        env.storage()
            .instance()
            .remove(&DataKey::PendingEpochConfig);
        EpochConfigApplied {
            epoch_length: c.epoch_length,
            epoch_budget: c.epoch_budget,
            rollover: c.rollover,
        }
        .publish(&env);
        Ok(())
    }

    pub fn get_epoch_config(env: Env) -> Option<EpochConfig> {
        env.storage().instance().get(&DataKey::EpochConfig)
    }

    pub fn get_pending_epoch_config(env: Env) -> Option<PendingEpochConfig> {
        env.storage().instance().get(&DataKey::PendingEpochConfig)
    }

    /// Amount approved in the epoch starting at `epoch_start`.
    pub fn get_epoch_approved(env: Env, epoch_start: u64) -> i128 {
        Self::epoch_approved(&env, epoch_start)
    }

    /// Remaining approval budget in the current epoch, or `None` when no
    /// epoch budget is configured.
    pub fn get_epoch_remaining(env: Env) -> Option<i128> {
        EpochState::load(&env).map(|e| e.available - e.approved)
    }

    // ── Attester: payout recipient ──

    /// Designate the address that receives this attester's payouts.
    /// Requires attester auth; the attester must be allowlisted (a
    /// suspended attester cannot redirect funds). Takes effect after
    /// `PAYOUT_ADDRESS_DELAY`.
    pub fn set_payout_address(
        env: Env,
        attester: Address,
        recipient: Address,
    ) -> Result<(), Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        let registry = AttesterRegistryClient::new(&env, &Self::attester_registry(&env)?);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }
        let now = env.ledger().timestamp();
        let effective_at = now + PAYOUT_ADDRESS_DELAY;
        let binding = PayoutBinding {
            current: Self::resolve_payout(&env, &attester, now),
            pending: Some(recipient.clone()),
            effective_at,
        };
        env.storage()
            .persistent()
            .set(&DataKey::PayoutBinding(attester.clone()), &binding);
        PayoutAddressScheduled {
            attester,
            recipient,
            effective_at,
        }
        .publish(&env);
        Ok(())
    }

    /// The address that would receive `attester`'s payout right now.
    pub fn get_payout_address(env: Env, attester: Address) -> Address {
        Self::resolve_payout(&env, &attester, env.ledger().timestamp()).unwrap_or(attester)
    }

    pub fn get_payout_binding(env: Env, attester: Address) -> Option<PayoutBinding> {
        env.storage()
            .persistent()
            .get(&DataKey::PayoutBinding(attester))
    }

    // ── Attester: claim payout ──

    /// Claim payout for an approved work item. Requires the approved
    /// attester's auth. Pays the attester's bound payout address (see
    /// `set_payout_address`), defaulting to the attester itself. Enforces replay protection, per-attester cap,
    /// and pool balance checks. Transfers tokens on success.
    pub fn claim(env: Env, work_item_id: BytesN<32>) -> Result<(), Error> {
        Self::require_not_paused(&env)?;

        let work_item = Self::open_work_item(&env, &work_item_id)?;

        let attester = work_item.attester.clone();
        attester.require_auth();

        // Verify attester is still allowlisted (may have been suspended
        // since approval).
        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }
        // A registry without probation support leaves payouts open.
        if let Ok(Ok(false)) = registry.try_is_payout_eligible(&attester) {
            return Err(Error::PayoutNotEligible);
        }

        // Enforce per-attester cumulative cap.
        let max_per_attester: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxPerAttester)
            .unwrap_or(i128::MAX);
        let attester_claimed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AttesterTotalClaimed(attester.clone()))
            .unwrap_or(0);
        let new_attester_claimed = attester_claimed
            .checked_add(work_item.payout_amount)
            .ok_or(Error::ArithmeticOverflow)?;
        if new_attester_claimed > max_per_attester {
            return Err(Error::AttesterClaimCapExceeded);
        }

        // Verify pool has sufficient unallocated balance.
        let total_paid = Self::total_paid(&env);
        let new_paid = total_paid
            .checked_add(work_item.payout_amount)
            .ok_or(Error::ArithmeticOverflow)?;
        if new_paid > Self::total_deposited(&env) {
            return Err(Error::InsufficientPoolBalance);
        }

        // Custody check: refuse to pay out while accounting and the actual
        // token balance disagree (fee-on-transfer, clawback, rebasing).
        let rec = Self::reconciliation(&env)?;
        if Self::is_drift_detected(env.clone()) || rec.drift < 0 {
            return Err(Error::ReconciliationRequired);
        }

        // Execute token transfer and verify the observed balance delta.
        // On failure the transaction is rolled back.
        let token_addr = Self::token(&env)?;
        let pool_addr = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        let recipient = Self::get_payout_address(env.clone(), attester.clone());
        // Surface trustline / authorization failures as a typed error.
        if !matches!(
            token_client.try_transfer(&pool_addr, &recipient, &work_item.payout_amount),
            Ok(Ok(()))
        ) {
            return Err(Error::TransferFailed);
        }

        // Persist state updates after successful transfer.
        env.storage()
            .persistent()
            .set(&DataKey::WorkItemClaimed(work_item_id.clone()), &true);
        env.storage().instance().set(&DataKey::TotalPaid, &new_paid);
        env.storage().persistent().set(
            &DataKey::AttesterTotalClaimed(attester.clone()),
            &new_attester_claimed,
        );
        if env
            .storage()
            .persistent()
            .has(&DataKey::WorkItemV2(work_item_id.clone()))
        {
            Self::release_reservation(&env, work_item.payout_amount)?;
        }

        // The claimant pays to keep replay-protection and cap entries live.
        Self::bump_persistent(&env, &DataKey::WorkItem(work_item_id.clone()));
        Self::bump_persistent(&env, &DataKey::WorkItemClaimed(work_item_id.clone()));
        Self::bump_persistent(&env, &DataKey::AttesterTotalClaimed(attester.clone()));
        Self::bump_instance(&env);

        PayoutClaimed {
            work_item_id,
            attester,
            recipient,
            amount: work_item.payout_amount,
        }
        .publish(&env);

        Ok(())
    }

    // ── Pause / unpause ──

    /// Pause the contract, blocking `fund`, `withdraw`,
    /// `approve_work_item`, and `claim` until `unpause` is called.
    /// Requires admin auth.
    pub fn pause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        Paused { by: admin }.publish(&env);
        Ok(())
    }

    /// Resume normal operation after a `pause`. Requires admin auth.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        Unpaused { by: admin }.publish(&env);
        Ok(())
    }

    /// Preflight: whether `address` can currently receive the pool token.
    /// For Stellar Asset Contracts this checks that a trustline exists and
    /// is authorized by the issuer. For other tokens, falls back to whether
    /// a balance can be read for the address.
    pub fn can_receive(env: Env, address: Address) -> Result<bool, Error> {
        let token_addr = Self::token(&env)?;
        match token::StellarAssetClient::new(&env, &token_addr).try_authorized(&address) {
            Ok(Ok(authorized)) => Ok(authorized),
            _ => Ok(matches!(
                token::Client::new(&env, &token_addr).try_balance(&address),
                Ok(Ok(_))
            )),
        }
    }

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    // ── Private helpers ──

    fn approve(
        env: &Env,
        work_item_id: BytesN<32>,
        attester: Address,
        payout_amount: i128,
        ttl_secs: u64,
    ) -> Result<(), Error> {
        Self::approver(env)?.require_auth();
        Self::require_not_paused(env)?;

        if payout_amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }

        // Reject duplicate approval; cancelled records stay in storage as
        // tombstones, so a cancelled id is rejected here too.
        if Self::is_cancelled(env, &work_item_id) {
            return Err(Error::WorkItemCancelled);
        }
        if Self::has_work_item(env, &work_item_id) {
            return Err(Error::WorkItemAlreadyApproved);
        }

        // Verify attester is currently allowlisted.
        let registry_id = Self::attester_registry(env)?;
        let registry = AttesterRegistryClient::new(env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
        }

        // Enforce per-claim cap at approval time.
        let max_per_claim: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxPerClaim)
            .unwrap_or(i128::MAX);
        if payout_amount > max_per_claim {
            return Err(Error::PayoutCapExceeded);
        }

        let approved_at = env.ledger().timestamp();
        let work_item = WorkItem {
            attester: attester.clone(),
            payout_amount,
            approved_at,
            expires_at: approved_at
                .checked_add(ttl_secs)
                .ok_or(Error::ArithmeticOverflow)?,
        };

        env.storage()
            .persistent()
            .set(&DataKey::WorkItemV2(work_item_id.clone()), &work_item);

        let reserved = Self::total_reserved(env)
            .checked_add(payout_amount)
            .ok_or(Error::ArithmeticOverflow)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalReserved, &reserved);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        WorkItemApproved {
            work_item_id,
            attester,
            payout_amount,
        }
        .publish(env);

        Ok(())
    }

    fn has_work_item(env: &Env, id: &BytesN<32>) -> bool {
        let store = env.storage().persistent();
        store.has(&DataKey::WorkItemV2(id.clone())) || store.has(&DataKey::WorkItem(id.clone()))
    }

    /// Load a work item, upgrading a legacy record on read.
    fn load_work_item(env: &Env, id: &BytesN<32>) -> Result<WorkItem, Error> {
        let store = env.storage().persistent();
        if let Some(item) = store.get::<_, WorkItem>(&DataKey::WorkItemV2(id.clone())) {
            return Ok(item);
        }
        let legacy: LegacyWorkItem = store
            .get(&DataKey::WorkItem(id.clone()))
            .ok_or(Error::WorkItemNotApproved)?;
        Ok(WorkItem {
            attester: legacy.attester,
            payout_amount: legacy.payout_amount,
            approved_at: legacy.approved_at,
            expires_at: legacy.approved_at.saturating_add(DEFAULT_APPROVAL_TTL_SECS),
        })
    }

    /// Load a work item that is still claimable or cancellable: approved,
    /// not claimed, not cancelled, not expired.
    fn open_work_item(env: &Env, id: &BytesN<32>) -> Result<WorkItem, Error> {
        let item = Self::load_work_item(env, id)?;
        if Self::is_claimed(env, id) {
            return Err(Error::WorkItemAlreadyClaimed);
        }
        if Self::is_cancelled(env, id) {
            return Err(Error::WorkItemCancelled);
        }
        if Self::is_swept(env, id) || env.ledger().timestamp() >= item.expires_at {
            return Err(Error::WorkItemExpired);
        }
        Ok(item)
    }

    fn is_claimed(env: &Env, id: &BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::WorkItemClaimed(id.clone()))
            .unwrap_or(false)
    }

    fn is_cancelled(env: &Env, id: &BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::WorkItemCancelled(id.clone()))
    }

    fn is_swept(env: &Env, id: &BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::WorkItemSwept(id.clone()))
            .unwrap_or(false)
    }

    /// Decrease `TotalReserved`, flooring at zero: legacy items were never
    /// added to the reservation counter.
    fn release_reservation(env: &Env, amount: i128) -> Result<(), Error> {
        let reserved = Self::total_reserved(env)
            .checked_sub(amount)
            .ok_or(Error::ArithmeticOverflow)?
            .max(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalReserved, &reserved);
        Ok(())
    }

    fn reconciliation(env: &Env) -> Result<Reconciliation, Error> {
        let accounted = Self::total_deposited(env)
            .checked_sub(Self::total_paid(env))
            .ok_or(Error::ArithmeticOverflow)?;
        let token_client = token::Client::new(env, &Self::token(env)?);
        let actual_balance = token_client.balance(&env.current_contract_address());
        let drift = actual_balance
            .checked_sub(accounted)
            .ok_or(Error::ArithmeticOverflow)?;
        Ok(Reconciliation {
            accounted,
            actual_balance,
            drift,
        })
    }

    fn total_deposited(env: &Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0)
    }

    fn total_paid(env: &Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalPaid)
            .unwrap_or(0)
    }

    fn total_reserved(env: &Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalReserved)
            .unwrap_or(0)
    }

    fn default_ttl(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::DefaultApprovalTtl)
            .unwrap_or(DEFAULT_APPROVAL_TTL_SECS)
    }

    fn max_ttl(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MaxApprovalTtl)
            .unwrap_or(DEFAULT_MAX_APPROVAL_TTL_SECS)
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn approver(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Approver)
            .ok_or(Error::NotInitialized)
    }

    fn token(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(Error::NotInitialized)
    }

    fn attester_registry(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::AttesterRegistry)
            .ok_or(Error::NotInitialized)
    }

    fn resolve_payout(env: &Env, attester: &Address, now: u64) -> Option<Address> {
        let b: PayoutBinding = env
            .storage()
            .persistent()
            .get(&DataKey::PayoutBinding(attester.clone()))?;
        match b.pending {
            Some(p) if now >= b.effective_at => Some(p),
            _ => b.current,
        }
    }

    fn epoch_approved(env: &Env, epoch_start: u64) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::EpochApproved(epoch_start))
            .unwrap_or(0)
    }

    fn require_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }
}

/// Per-call approval state shared by single and batch approval.
struct ApprovalCtx {
    attester_registry: Address,
    attestation_registry: Address,
    max_per_claim: i128,
    allowlisted: Map<Address, bool>,
    epoch: Option<EpochState>,
}

impl ApprovalCtx {
    fn load(env: &Env) -> Result<Self, Error> {
        Ok(Self {
            attester_registry: IncentivePool::attester_registry(env)?,
            attestation_registry: env
                .storage()
                .instance()
                .get(&DataKey::AttestationRegistry)
                .ok_or(Error::AttestationRegistryNotSet)?,
            max_per_claim: env
                .storage()
                .instance()
                .get(&DataKey::MaxPerClaim)
                .unwrap_or(i128::MAX),
            allowlisted: Map::new(env),
            epoch: EpochState::load(env),
        })
    }

    fn commit(&self, env: &Env) {
        if let Some(e) = &self.epoch {
            env.storage()
                .persistent()
                .set(&DataKey::EpochApproved(e.start), &e.approved);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }
}

/// Current-epoch accounting.
struct EpochState {
    start: u64,
    available: i128,
    approved: i128,
}

impl EpochState {
    fn load(env: &Env) -> Option<Self> {
        let c: EpochConfig = env.storage().instance().get(&DataKey::EpochConfig)?;
        let start = env.ledger().timestamp() / c.epoch_length * c.epoch_length;
        let mut available = c.epoch_budget;
        if c.rollover && start >= c.epoch_length {
            let prev = IncentivePool::epoch_approved(env, start - c.epoch_length);
            available = available.saturating_add((c.epoch_budget - prev).max(0));
        }
        Some(Self {
            start,
            available,
            approved: IncentivePool::epoch_approved(env, start),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
mod test;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod stateful_test;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod fuzz_test;
