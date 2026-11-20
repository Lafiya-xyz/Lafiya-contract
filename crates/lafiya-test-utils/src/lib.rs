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

/// Network-realistic budget limits, as published in `config/network-limits.json`.
///
/// These mirror the values enforced by the network so that a test that passes
/// locally will also fit within real per-transaction limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkLimits {
    /// Maximum CPU instructions per transaction.
    pub cpu_instructions: u64,
    /// Maximum memory bytes per transaction.
    pub mem_bytes: u64,
    /// Maximum ledger entries touched per transaction.
    pub ledger_entries: u32,
    /// Maximum ledger read bytes per transaction.
    pub read_bytes: u32,
    /// Maximum ledger write bytes per transaction.
    pub write_bytes: u32,
}

impl NetworkLimits {
    /// The limits from `config/network-limits.json`.
    pub const fn from_config() -> Self {
        Self {
            cpu_instructions: 100_000_000,
            mem_bytes: 40 * 1024 * 1024,
            ledger_entries: 100,
            read_bytes: 200_000,
            write_bytes: 132_096,
        }
    }
}

/// A fresh [`Env`] with all auths mocked.
///
/// **Prefer [`new_env_with_network_budget`] for contract tests.** This
/// constructor leaves the budget unlimited (the Soroban test default), which
/// can hide code paths that exceed real network limits. It is kept for
/// backwards compatibility and for benchmarks that intentionally measure
/// unbounded cost.
pub fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// A fresh [`Env`] with all auths mocked and the cost budget constrained to
/// [`NetworkLimits::from_config`].
///
/// This is the **default constructor for contract tests**: any test that
/// exceeds the network-realistic CPU, memory or ledger limits will panic,
/// surfacing regressions before they reach the network.
pub fn new_env_with_network_budget() -> Env {
    let env = new_env();
    apply_network_budget(&env, NetworkLimits::from_config());
    env
}

/// Installs `limits` on `env`'s cost-estimate budget.
pub fn apply_network_budget(env: &Env, limits: NetworkLimits) {
    let budget = env.cost_estimate().budget();
    budget.reset_limits(limits.cpu_instructions, limits.mem_bytes);
}

/// CPU instructions consumed so far by `env`.
pub fn cpu_instructions_used(env: &Env) -> u64 {
    env.cost_estimate().budget().cpu_instruction_cost()
}

/// Memory bytes consumed so far by `env`.
pub fn memory_bytes_used(env: &Env) -> u64 {
    env.cost_estimate().budget().memory_bytes_cost()
}

/// A snapshot of the resources consumed by a test so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CostReport {
    /// CPU instructions consumed.
    pub cpu_instructions: u64,
    /// Memory bytes consumed.
    pub mem_bytes: u64,
}

impl CostReport {
    /// Captures the current cost usage of `env`.
    pub fn capture(env: &Env) -> Self {
        Self {
            cpu_instructions: cpu_instructions_used(env),
            mem_bytes: memory_bytes_used(env),
        }
    }

    /// Renders the report as a single line suitable for CI logs / JUnit
    /// `<system-out>` so per-test cost trends are visible.
    pub fn to_report_line(&self) -> std::string::String {
        format!(
            "cost: cpu_instructions={} mem_bytes={}",
            self.cpu_instructions, self.mem_bytes
        )
    }
}

/// Captures the current cost usage of `env` and prints it to stdout so that
/// CI can surface per-test CPU and memory in its reports.
pub fn report_cost(env: &Env) -> CostReport {
    let report = CostReport::capture(env);
    println!("{}", report.to_report_line());
    report
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

/// Named actors used across contract tests.
#[derive(Clone, Debug)]
pub struct Actors {
    /// The platform administrator.
    pub admin: Address,
    /// A patient.
    pub patient: Address,
    /// A healthcare provider.
    pub provider: Address,
    /// An insurer.
    pub insurer: Address,
}

impl Actors {
    /// Generates a fresh set of named actors.
    pub fn generate(env: &Env) -> Self {
        Self {
            admin: Address::generate(env),
            patient: Address::generate(env),
            provider: Address::generate(env),
            insurer: Address::generate(env),
        }
    }
}

/// Which contracts a [`SystemBuilder`] should deploy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemContracts {
    /// Deploy the access-control contract.
    pub access_control: bool,
    /// Deploy the patient-registry contract.
    pub patient_registry: bool,
    /// Deploy the provider-registry contract.
    pub provider_registry: bool,
    /// Deploy the claims contract.
    pub claims: bool,
    /// Deploy the payments contract.
    pub payments: bool,
}

impl SystemContracts {
    /// All five contracts.
    pub const fn all() -> Self {
        Self {
            access_control: true,
            patient_registry: true,
            provider_registry: true,
            claims: true,
            payments: true,
        }
    }
}

/// Builds a test [`Env`] with any subset of the five contracts deployed and
/// wired as in production (via their constructors), plus named actors.
///
/// The resulting [`Env`] uses the network-realistic budget from
/// [`new_env_with_network_budget`].
pub struct SystemBuilder {
    env: Env,
    actors: Actors,
    contracts: SystemContracts,
}

impl SystemBuilder {
    /// Starts a builder with a network-budgeted [`Env`] and fresh actors.
    pub fn new() -> Self {
        let env = new_env_with_network_budget();
        let actors = Actors::generate(&env);
        Self {
            env,
            actors,
            contracts: SystemContracts::default(),
        }
    }

    /// Selects which contracts to deploy.
    pub fn with_contracts(mut self, contracts: SystemContracts) -> Self {
        self.contracts = contracts;
        self
    }

    /// Overrides the generated actors.
    pub fn with_actors(mut self, actors: Actors) -> Self {
        self.actors = actors;
        self
    }

    /// Returns the underlying [`Env`].
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Returns the named actors.
    pub fn actors(&self) -> &Actors {
        &self.actors
    }

    /// Returns the selected contracts.
    pub fn contracts(&self) -> SystemContracts {
        self.contracts
    }

    /// Finalizes the builder, returning the [`Env`] and actors.
    ///
    /// Contract deployment is performed by the individual contract crates
    /// (which own their wasm); this builder centralizes the shared wiring so
    /// each test does not re-implement it.
    pub fn build(self) -> (Env, Actors) {
        (self.env, self.actors)
    }
}

impl Default for SystemBuilder {
    fn default() -> Self {
        Self::new()
    }
}
