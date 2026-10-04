//! Shared security-critical code for Lafiya contracts.
//!
//! This crate provides reusable modules for admin management, pause control, and TTL
//! management. Each contract supplies its own `DataKey` enum through a trait,
//! ensuring that key order (ABI-critical for Soroban) remains per-contract.

#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod admin;
pub mod pausable;
pub mod ttl;

pub use admin::Admin;
pub use pausable::Pausable;
pub use ttl::{extend_instance_ttl, INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD};

// Re-export commonly used soroban types
pub use soroban_sdk::{Address, Env};
