#![no_std]

//! Shared cross-contract interface traits for the Lafiya contracts.
//!
//! This crate is the single source of truth for cross-contract client traits.
//! Consuming contracts (e.g. `incentive-pool`, `attestation-registry`) depend on
//! this crate instead of hand-copying `#[contractclient]` traits, so a change to
//! a provider's signature becomes a compile error (or a conformance failure)
//! rather than a runtime host error.
//!
//! Provider contracts should also depend on this crate and assert that their
//! exported interface matches the shared trait (see the `impl` checks in each
//! provider crate).

use soroban_sdk::{contractclient, Address, Env, Vec};

/// Client trait for the `attestation-registry` contract.
///
/// Mirrors the public interface of `contracts/attestation-registry`. The
/// provider crate must implement this trait (or otherwise assert conformance)
/// so that signature drift is caught at build time.
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistry {
    /// Register a new attester with the given metadata.
    fn register_attester(env: Env, attester: Address, metadata: Vec<u8>);

    /// Revoke an attester's registration.
    fn revoke_attester(env: Env, attester: Address);

    /// Returns `true` if `attester` is currently registered and active.
    fn is_attester(env: Env, attester: Address) -> bool;

    /// Returns `true` if the given session is valid for `attester`.
    fn is_session_valid(env: Env, attester: Address, session_id: u64) -> bool;

    /// Create a new session for `attester`, returning the session id.
    fn create_session(env: Env, attester: Address) -> u64;

    /// Invalidate an existing session.
    fn invalidate_session(env: Env, attester: Address, session_id: u64);
}

/// Client trait for the `incentive-pool` contract.
///
/// Mirrors the public interface of `contracts/incentive-pool`. Consuming
/// contracts use this trait instead of a hand-copied definition.
#[contractclient(name = "IncentivePoolClient")]
pub trait IncentivePool {
    /// Deposit `amount` of the pool token from `from`.
    fn deposit(env: Env, from: Address, amount: i128);

    /// Withdraw `amount` of the pool token to `to`.
    fn withdraw(env: Env, to: Address, amount: i128);

    /// Claim accrued incentives for `claimant`.
    fn claim(env: Env, claimant: Address) -> i128;

    /// Returns the claimable balance for `claimant`.
    fn claimable(env: Env, claimant: Address) -> i128;

    /// Returns the total balance held by the pool.
    fn total_balance(env: Env) -> i128;
}

/// Client trait for the `health-records` contract.
///
/// Mirrors the public interface of `contracts/health-records`. Consuming
/// contracts use this trait instead of a hand-copied definition.
#[contractclient(name = "HealthRecordsClient")]
pub trait HealthRecords {
    /// Grant `grantee` read access to the record owned by `owner`.
    fn grant_access(env: Env, owner: Address, grantee: Address);

    /// Revoke `grantee`'s read access to the record owned by `owner`.
    fn revoke_access(env: Env, owner: Address, grantee: Address);

    /// Returns `true` if `grantee` may read `owner`'s record.
    fn has_access(env: Env, owner: Address, grantee: Address) -> bool;

    /// Store an encrypted record blob for `owner`, returning its id.
    fn put_record(env: Env, owner: Address, payload: Vec<u8>) -> u64;

    /// Fetch the encrypted record blob with `record_id` for `owner`.
    fn get_record(env: Env, owner: Address, record_id: u64) -> Vec<u8>;
}

pub use AttesterRegistryClient;
pub use HealthRecordsClient;
pub use IncentivePoolClient;