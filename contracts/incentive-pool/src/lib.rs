#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![warn(missing_docs)]

//! # Incentive Pool
//!
//! Bounded USDC payout/escrow contract for the M2 milestone.
//! Donors fund the pool, an approver authorizes work items, and
//! approved allowlisted attesters claim per-item payouts with
//! per-attestation replay protection.

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, token,
    Address, BytesN, Env,
};

/// The subset of the `attester-registry` contract this crate calls.
/// Kept as a trait interface (rather than a direct crate dependency)
/// so that `attester-registry`'s own contract implementation never
/// links into this crate's wasm — only the typed cross-contract
/// call it generates does.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
}

const SCHEMA_VERSION: u32 = 1;

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

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
pub struct PayoutClaimed {
    #[topic]
    pub work_item_id: BytesN<32>,
    #[topic]
    pub attester: Address,
    pub amount: i128,
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
}

// ──────────────────────────── Contract ────────────────────────────

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
        env.storage()
            .instance()
            .set(&DataKey::Approver, &approver);
        env
            .storage()
            .instance()
            .set(&DataKey::Token, &token);
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
        env.storage()
            .instance()
            .set(&DataKey::TotalPaid, &0_i128);
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
        env.storage()
            .persistent()
            .has(&DataKey::WorkItem(work_item_id))
    }

    /// Whether a work item has been claimed.
    pub fn is_work_item_claimed(env: Env, work_item_id: BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::WorkItemClaimed(work_item_id))
            .unwrap_or(false)
    }

    /// Return the work item details, if approved.
    pub fn get_work_item(env: Env, work_item_id: BytesN<32>) -> Option<WorkItem> {
        env.storage()
            .persistent()
            .get(&DataKey::WorkItem(work_item_id))
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
        env.storage()
            .instance()
            .set(&DataKey::MaxPerAttester, &max);
        Ok(())
    }

    // ── Admin: fund / withdraw ──

    /// Deposit `amount` tokens from the caller into the pool.
    /// Requires admin auth. The admin must have previously approved
    /// the pool contract to spend their tokens, or the transaction
    /// must include the inner `token.transfer` in the authorization tree.
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

        token_client.transfer(&admin, &pool_addr, &amount);

        let deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &(deposited + amount));

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        PoolFunded {
            funder: admin,
            amount,
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

        let total_paid: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalPaid)
            .unwrap_or(0);
        let total_deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        let available = total_deposited - total_paid;
        if amount > available {
            return Err(Error::InsufficientPoolBalance);
        }

        let token_addr = Self::token(&env)?;
        let pool_addr = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        token_client.transfer(&pool_addr, &to, &amount);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        PoolWithdrawn { to, amount }.publish(&env);

        Ok(())
    }

    // ── Approver: approve work items ──

    /// Approve a work item for payout. Requires the approver's auth.
    /// The `payout_amount` must be positive and must not exceed the
    /// per-claim cap. The attester must be allowlisted.
    pub fn approve_work_item(
        env: Env,
        work_item_id: BytesN<32>,
        attester: Address,
        payout_amount: i128,
    ) -> Result<(), Error> {
        Self::approver(&env)?.require_auth();
        Self::require_not_paused(&env)?;

        if payout_amount <= 0 {
            return Err(Error::NonPositiveAmount);
        }

        // Reject duplicate approval.
        if env
            .storage()
            .persistent()
            .has(&DataKey::WorkItem(work_item_id.clone()))
        {
            return Err(Error::WorkItemAlreadyApproved);
        }

        // Verify attester is currently allowlisted.
        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
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

        let work_item = WorkItem {
            attester: attester.clone(),
            payout_amount,
            approved_at: env.ledger().timestamp(),
        };

        env.storage().persistent().set(
            &DataKey::WorkItem(work_item_id.clone()),
            &work_item,
        );

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        WorkItemApproved {
            work_item_id,
            attester,
            payout_amount,
        }
        .publish(&env);

        Ok(())
    }

    // ── Attester: claim payout ──

    /// Claim payout for an approved work item. Requires the approved
    /// attester's auth. Enforces replay protection, per-attester cap,
    /// and pool balance checks. Transfers tokens on success.
    pub fn claim(env: Env, work_item_id: BytesN<32>) -> Result<(), Error> {
        Self::require_not_paused(&env)?;

        let work_item: WorkItem = env
            .storage()
            .persistent()
            .get(&DataKey::WorkItem(work_item_id.clone()))
            .ok_or(Error::WorkItemNotApproved)?;

        let claimed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::WorkItemClaimed(work_item_id.clone()))
            .unwrap_or(false);
        if claimed {
            return Err(Error::WorkItemAlreadyClaimed);
        }

        let attester = work_item.attester.clone();
        attester.require_auth();

        // Verify attester is still allowlisted (may have been suspended
        // since approval).
        let registry_id = Self::attester_registry(&env)?;
        let registry = AttesterRegistryClient::new(&env, &registry_id);
        if !registry.is_attester(&attester) {
            return Err(Error::AttesterNotAllowlisted);
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
        if attester_claimed + work_item.payout_amount > max_per_attester {
            return Err(Error::AttesterClaimCapExceeded);
        }

        // Verify pool has sufficient unallocated balance.
        let total_paid: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalPaid)
            .unwrap_or(0);
        let total_deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        if total_paid + work_item.payout_amount > total_deposited {
            return Err(Error::InsufficientPoolBalance);
        }

        // Execute token transfer. On failure the transaction is
        // rolled back, preserving consistent state.
        let token_addr = Self::token(&env)?;
        let pool_addr = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        token_client.transfer(&pool_addr, &attester, &work_item.payout_amount);

        // Persist state updates after successful transfer.
        env.storage().persistent().set(
            &DataKey::WorkItemClaimed(work_item_id.clone()),
            &true,
        );
        env.storage().instance().set(
            &DataKey::TotalPaid,
            &(total_paid + work_item.payout_amount),
        );
        env.storage().persistent().set(
            &DataKey::AttesterTotalClaimed(attester.clone()),
            &(attester_claimed + work_item.payout_amount),
        );

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        PayoutClaimed {
            work_item_id,
            attester,
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

    /// Whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    // ── Private helpers ──

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
