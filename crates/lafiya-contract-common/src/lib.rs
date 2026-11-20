//! Shared security-critical code for Lafiya contracts.
//!
//! This crate provides reusable modules for admin management, pause control, and TTL
//! management. Each contract supplies its own `DataKey` enum through a trait,
//! ensuring that key order (ABI-critical for Soroban) remains per-contract.

#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod admin;
pub mod migration;
pub mod pausable;
pub mod storage;
pub mod ttl;

pub use admin::Admin;
pub use pausable::Pausable;
pub use ttl::{extend_instance_ttl, INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD};

// Re-export commonly used soroban types
pub use soroban_sdk::{Address, Env};

/// Default lifetime (in ledgers) for a pending admin proposal before it expires.
///
/// Contracts that implement two-step admin transfer may use this constant to
/// bound how long a proposed admin remains acceptable. It is expressed in
/// ledgers, matching Soroban's TTL semantics.
pub const ADMIN_PROPOSAL_LIFETIME: u32 = 17_280; // ~1 day at 5s/ledger

/// Governance event topics shared across contracts.
///
/// These are the canonical topic symbols emitted by the common governance
/// modules. Contracts should emit events using these topics so that off-chain
/// indexers and the conformance snapshots stay consistent.
pub mod events {
    use soroban_sdk::{symbol_short, Symbol};

    /// Topic emitted when a new admin is proposed (two-step transfer).
    #[inline]
    pub fn admin_proposed() -> Symbol {
        symbol_short!("adm_prop")
    }

    /// Topic emitted when a proposed admin accepts the role.
    #[inline]
    pub fn admin_accepted() -> Symbol {
        symbol_short!("adm_acc")
    }

    /// Topic emitted when the admin is changed directly (single-step).
    #[inline]
    pub fn admin_changed() -> Symbol {
        symbol_short!("adm_chg")
    }

    /// Topic emitted when the contract is paused.
    #[inline]
    pub fn paused() -> Symbol {
        symbol_short!("paused")
    }

    /// Topic emitted when the contract is unpaused.
    #[inline]
    pub fn unpaused() -> Symbol {
        symbol_short!("unpaused")
    }
}

/// Compute the ledger at which a proposal created at `created_at` expires.
///
/// Returns `created_at + ADMIN_PROPOSAL_LIFETIME`, saturating on overflow so
/// that a proposal never wraps around to an already-expired value.
#[inline]
pub fn proposal_expiry(created_at: u32) -> u32 {
    created_at.saturating_add(ADMIN_PROPOSAL_LIFETIME)
}

/// Returns `true` when a proposal created at `created_at` is still valid at
/// `now` (i.e. `now < proposal_expiry(created_at)`).
#[inline]
pub fn proposal_is_live(created_at: u32, now: u32) -> bool {
    now < proposal_expiry(created_at)
}