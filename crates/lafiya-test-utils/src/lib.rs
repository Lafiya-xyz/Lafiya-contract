//! Shared fixtures, builders and assertion helpers for Lafiya contract tests.
//!
//! This crate is a **dev-dependency only**. It must never appear in a
//! contract's `[dependencies]`, so it cannot affect production wasm.
//!
//! ```toml
//! [dev-dependencies]
//! lafiya-test-utils = { path = "../../crates/lafiya-test-utils" }
//! ```

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Event};

/// A fresh [`Env`] with all auths mocked.
pub fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Generates a random address.
pub fn addr(env: &Env) -> Address {
    Address::generate(env)
}

/// Moves the ledger sequence number forward by `n`.
pub fn advance_ledgers(env: &Env, n: u32) {
    env.ledger().with_mut(|l| l.sequence_number += n);
}

/// Moves the ledger timestamp forward by `seconds`.
pub fn advance_time(env: &Env, seconds: u64) {
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + seconds);
}

/// Registers a mock Stellar asset token and returns its address and a client
/// that can mint. The token admin is a freshly generated address.
pub fn register_token<'a>(env: &'a Env) -> (Address, StellarAssetClient<'a>) {
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    let client = StellarAssetClient::new(env, &token);
    (token, client)
}

/// Like [`register_token`], and mints `amount` to `to`.
pub fn register_funded_token<'a>(
    env: &'a Env,
    to: &Address,
    amount: i128,
) -> (Address, StellarAssetClient<'a>) {
    let (token, client) = register_token(env);
    client.mint(to, &amount);
    (token, client)
}

/// Asserts that the events published by `contract` so far are exactly
/// `expected`, in order.
pub fn assert_events<E: Event>(env: &Env, contract: &Address, expected: &[E]) {
    let want: std::vec::Vec<_> = expected.iter().map(|e| e.to_xdr(env, contract)).collect();
    assert_eq!(env.events().all().filter_by_contract(contract), want);
}

/// Asserts that the most recent event published (by any contract) is `expected`.
pub fn assert_last_event<E: Event>(env: &Env, contract: &Address, expected: &E) {
    assert_eq!(
        env.events().all().events().last(),
        Some(&expected.to_xdr(env, contract)),
    );
}
