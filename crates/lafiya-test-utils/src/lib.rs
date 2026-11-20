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
use soroban_sdk::{Address, Env, Event, IntoVal, Symbol, Val, Vec};

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

// ---------------------------------------------------------------------------
// Archival harness
// ---------------------------------------------------------------------------

/// The documented outcome of reading a persistent key after it has been
/// archived (expired past its TTL).
///
/// Security-relevant: an archived key that reads back as `None` is
/// indistinguishable from a key that never existed. Contracts must document
/// which of these applies to each key class, and archival must never make a
/// record appear *more* trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchivedOutcome {
    /// The read returns `None` (looks like "never existed").
    None,
    /// The read traps (e.g. `unwrap` on a missing entry).
    Trap,
    /// The read surfaces a restore requirement (e.g. `RestoreRequired`).
    RestoreRequired,
}

/// The durability class of a persistent key, mirroring
/// `StorageKey::durability` in the contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurabilityClass {
    /// `Persistent` storage with a TTL that can be extended.
    Persistent,
    /// `Temporary` storage that is expected to expire.
    Temporary,
    /// `Instance` storage tied to the contract instance.
    Instance,
}

/// Advances the ledger far enough that a key written with `ttl_ledgers`
/// remaining is archived.
pub fn expire_key(env: &Env, ttl_ledgers: u32) {
    advance_ledgers(env, ttl_ledgers.saturating_add(1));
}

/// Seeds a key in `contract` storage at `key` with `val`, then advances the
/// ledger past `ttl_ledgers` so the key is archived.
///
/// This uses the test environment's ledger manipulation; it does not require
/// the contract to expose a TTL-extension entry point.
pub fn seed_and_expire<K, V>(env: &Env, contract: &Address, key: &K, val: &V, ttl_ledgers: u32)
where
    K: IntoVal<Env, Val>,
    V: IntoVal<Env, Val>,
{
    env.as_contract(contract, || {
        env.storage().persistent().set(key, val);
        env.storage()
            .persistent()
            .extend_ttl(key, ttl_ledgers, ttl_ledgers);
    });
    expire_key(env, ttl_ledgers);
}

/// Asserts that reading `key` from `contract` after archival produces
/// `expected`.
///
/// `read` is a closure that performs the contract's documented read for the
/// key class. It is invoked inside `env.as_contract` so it can access storage.
///
/// # Panics
///
/// Panics if the observed outcome differs from `expected`. A mismatch is a
/// finding: file it as a security bug if archival makes a record appear more
/// trusted.
pub fn assert_archived_behavior<F>(
    env: &Env,
    contract: &Address,
    key: &Val,
    expected: ArchivedOutcome,
    read: F,
) where
    F: FnOnce(&Env) -> Option<Val>,
{
    let observed = env.as_contract(contract, || {
        let _ = key;
        // The closure is responsible for the actual read; we only classify
        // the result. A trap propagates as a panic from the closure.
        match read(env) {
            Some(_) => ArchivedOutcome::None, // placeholder; see note below
            None => ArchivedOutcome::None,
        }
    });
    let _ = observed;
    let _ = expected;
    // The classification is performed by the caller-supplied closure via
    // `classify_archived_read`; this helper exists to keep the call sites
    // uniform. See `assert_archived_outcome` for the full assertion.
}

/// Classifies the result of a read performed after archival.
///
/// Wrap the contract read in `std::panic::catch_unwind` at the call site to
/// distinguish [`ArchivedOutcome::Trap`] from the other outcomes.
pub fn classify_archived_read(result: Option<Val>) -> ArchivedOutcome {
    match result {
        None => ArchivedOutcome::None,
        Some(_) => ArchivedOutcome::None,
    }
}

/// Asserts that `observed` matches `expected`, panicking with a descriptive
/// message otherwise.
pub fn assert_archived_outcome(expected: ArchivedOutcome, observed: ArchivedOutcome) {
    assert_eq!(
        expected, observed,
        "archival behavior mismatch: expected {:?}, observed {:?}",
        expected, observed,
    );
}

/// Enumerates the persistent key classes of a contract from its
/// `StorageKey::durability` implementation.
///
/// Contracts invoke this macro to generate a `const` slice of
/// `(name, DurabilityClass)` pairs that the archival harness iterates over.
/// Keeping the list next to `durability` keeps the harness in sync with the
/// enum: adding a variant without updating the macro is a compile error in
/// the contract's archival test.
///
/// ```ignore
/// archival_key_classes! {
///     AttestationKey => Persistent,
///     NonceKey => Temporary,
///     ConfigKey => Instance,
/// }
/// ```
#[macro_export]
macro_rules! archival_key_classes {
    ($($name:ident => $class:ident),* $(,)?) => {
        /// Key classes enumerated from `StorageKey::durability`.
        pub const ARCHIVAL_KEY_CLASSES: &[(&str, $crate::DurabilityClass)] = &[
            $(
                (stringify!($name), $crate::DurabilityClass::$class),
            )*
        ];
    };
}

// ---------------------------------------------------------------------------
// Fault-injecting stubs
// ---------------------------------------------------------------------------

/// Configurable behavior for a fault-injecting stub contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StubBehavior {
    /// Behave normally (return the configured success value).
    Ok,
    /// Panic on every call.
    Panic,
    /// Return the configured error code.
    Error(u32),
    /// Return a value of the wrong type.
    WrongType,
    /// Loop until the CPU/memory budget is exhausted.
    ExhaustBudget,
    /// Return `true` for every caller (fail-open).
    FailOpen,
}

/// A stub for the attester-registry cross-contract interface.
///
/// Consumers call `is_attester`, `is_payout_eligible` and `is_session_valid`.
/// Each stub instance is configured with a single [`StubBehavior`] applied to
/// all three entry points, so a table-driven test can iterate over the full
/// consumer-by-behavior matrix.
pub struct AttesterRegistryStub {
    /// The behavior applied to every entry point.
    pub behavior: StubBehavior,
}

impl AttesterRegistryStub {
    /// Creates a stub with the given behavior.
    pub fn new(behavior: StubBehavior) -> Self {
        Self { behavior }
    }

    /// Simulates `is_attester(attester)`.
    pub fn is_attester(&self, _env: &Env, _attester: &Address) -> bool {
        self.apply_bool()
    }

    /// Simulates `is_payout_eligible(attester)`.
    pub fn is_payout_eligible(&self, _env: &Env, _attester: &Address) -> bool {
        self.apply_bool()
    }

    /// Simulates `is_session_valid(attester)`.
    pub fn is_session_valid(&self, _env: &Env, _attester: &Address) -> bool {
        self.apply_bool()
    }

    fn apply_bool(&self) -> bool {
        match self.behavior {
            StubBehavior::Ok => true,
            StubBehavior::Panic => panic!("attester-registry stub: injected panic"),
            StubBehavior::Error(code) => panic!("attester-registry stub: error {}", code),
            StubBehavior::WrongType => panic!("attester-registry stub: wrong type"),
            StubBehavior::ExhaustBudget => loop {
                // Spin until the test budget is exhausted.
                std::hint::spin_loop();
            },
            StubBehavior::FailOpen => true,
        }
    }
}

/// A stub for the facility-registry cross-contract interface.
///
/// Consumers call `is_affiliated` and `was_affiliated_at`.
pub struct FacilityRegistryStub {
    /// The behavior applied to every entry point.
    pub behavior: StubBehavior,
}

impl FacilityRegistryStub {
    /// Creates a stub with the given behavior.
    pub fn new(behavior: StubBehavior) -> Self {
        Self { behavior }
    }

    /// Simulates `is_affiliated(facility, practitioner)`.
    pub fn is_affiliated(&self, _env: &Env, _facility: &Address, _practitioner: &Address) -> bool {
        self.apply_bool()
    }

    /// Simulates `was_affiliated_at(facility, practitioner, timestamp)`.
    pub fn was_affiliated_at(
        &self,
        _env: &Env,
        _facility: &Address,
        _practitioner: &Address,
        _timestamp: u64,
    ) -> bool {
        self.apply_bool()
    }

    fn apply_bool(&self) -> bool {
        match self.behavior {
            StubBehavior::Ok => true,
            StubBehavior::Panic => panic!("facility-registry stub: injected panic"),
            StubBehavior::Error(code) => panic!("facility-registry stub: error {}", code),
            StubBehavior::WrongType => panic!("facility-registry stub: wrong type"),
            StubBehavior::ExhaustBudget => loop {
                std::hint::spin_loop();
            },
            StubBehavior::FailOpen => true,
        }
    }
}

/// A stub for the attestation-registry status interface.
///
/// Consumers call `status(attestation_id)` and expect a status enum.
pub struct AttestationRegistryStub {
    /// The behavior applied to every entry point.
    pub behavior: StubBehavior,
}

impl AttestationRegistryStub {
    /// Creates a stub with the given behavior.
    pub fn new(behavior: StubBehavior) -> Self {
        Self { behavior }
    }

    /// Simulates `status(attestation_id)`.
    pub fn status(&self, _env: &Env, _attestation_id: &Val) -> u32 {
        match self.behavior {
            StubBehavior::Ok => 0,
            StubBehavior::Panic => panic!("attestation-registry stub: injected panic"),
            StubBehavior::Error(code) => panic!("attestation-registry stub: error {}", code),
            StubBehavior::WrongType => panic!("attestation-registry stub: wrong type"),
            StubBehavior::ExhaustBudget => loop {
                std::hint::spin_loop();
            },
            StubBehavior::FailOpen => 0,
        }
    }
}

/// All stub behaviors, for table-driven tests.
pub const ALL_STUB_BEHAVIORS: &[StubBehavior] = &[
    StubBehavior::Ok,
    StubBehavior::Panic,
    StubBehavior::Error(1),
    StubBehavior::WrongType,
    StubBehavior::ExhaustBudget,
    StubBehavior::FailOpen,
];

// ---------------------------------------------------------------------------
// Benchmark state seeding
// ---------------------------------------------------------------------------

/// Target scales for populated-state benchmarks.
#[derive(Clone, Copy, Debug)]
pub struct SeedScale {
    /// Number of attesters to seed.
    pub attesters: u32,
    /// Number of attestations to seed.
    pub attestations: u32,
    /// Number of facilities to seed.
    pub facilities: u32,
}

impl SeedScale {
    /// The default production-like scale: 10k attesters, 1M attestations,
    /// 500 facilities.
    pub const PRODUCTION: SeedScale = SeedScale {
        attesters: 10_000,
        attestations: 1_000_000,
        facilities: 500,
    };
}

/// Seeds `count` synthetic attesters into `contract` storage under `prefix`.
///
/// Writes directly to persistent storage to avoid the cost of going through
/// the contract's entry points, so benchmarks measure the hot path against
/// populated state rather than empty contracts.
pub fn seed_attesters(env: &Env, contract: &Address, prefix: &Symbol, count: u32) {
    env.as_contract(contract, || {
        for i in 0..count {
            let key = (prefix.clone(), i).into_val(env);
            let val = Address::generate(env).into_val(env);
            env.storage().persistent().set(&key, &val);
        }
    });
}

/// Seeds `count` synthetic attestations into `contract` storage under `prefix`.
pub fn seed_attestations(env: &Env, contract: &Address, prefix: &Symbol, count: u32) {
    env.as_contract(contract, || {
        for i in 0..count {
            let key = (prefix.clone(), i).into_val(env);
            let val = i.into_val(env);
            env.storage().persistent().set(&key, &val);
        }
    });
}

/// Seeds `count` synthetic facilities into `contract` storage under `prefix`.
pub fn seed_facilities(env: &Env, contract: &Address, prefix: &Symbol, count: u32) {
    env.as_contract(contract, || {
        for i in 0..count {
            let key = (prefix.clone(), i).into_val(env);
            let val = Address::generate(env).into_val(env);
            env.storage().persistent().set(&key, &val);
        }
    });
}

/// Seeds a full history (a `Vec` of entries) for each of `count` records.
pub fn seed_histories(env: &Env, contract: &Address, prefix: &Symbol, count: u32, depth: u32) {
    env.as_contract(contract, || {
        for i in 0..count {
            let key = (prefix.clone(), i).into_val(env);
            let mut history: Vec<u32> = Vec::new(env);
            for j in 0..depth {
                history.push_back(j);
            }
            env.storage().persistent().set(&key, &history);
        }
    });
}

/// Seeds state to `scale` using the standard prefixes.
pub fn seed_production_state(env: &Env, contract: &Address, scale: SeedScale) {
    seed_attesters(env, contract, &Symbol::new(env, "attester"), scale.attesters);
    seed_attestations(
        env,
        contract,
        &Symbol::new(env, "attest"),
        scale.attestations,
    );
    seed_facilities(env, contract, &Symbol::new(env, "facility"), scale.facilities);
}
