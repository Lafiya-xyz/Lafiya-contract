//! Typed storage access layer.
//!
//! Each contract implements [`StorageKey`] for its `DataKey` enum with an
//! exhaustive `match`, so every key is bound to exactly one [`Durability`]
//! class at compile time. All reads and writes then go through the functions
//! below, which route to the correct store and can never mix up instance,
//! persistent and temporary storage for a key.
//!
//! To add a key: add the `DataKey` variant (append only, ABI order matters),
//! add its arm to the contract's `StorageKey` impl, and add it to the
//! contract's storage test key list.

use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

/// The storage store a key lives in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Durability {
    Instance,
    Persistent,
    Temporary,
}

/// Binds a key type to its durability class.
pub trait StorageKey {
    fn durability(&self) -> Durability;
}

pub fn get<K, V>(env: &Env, key: &K) -> Option<V>
where
    K: StorageKey + IntoVal<Env, Val>,
    V: TryFromVal<Env, Val>,
{
    let s = env.storage();
    match key.durability() {
        Durability::Instance => s.instance().get(key),
        Durability::Persistent => s.persistent().get(key),
        Durability::Temporary => s.temporary().get(key),
    }
}

pub fn set<K, V>(env: &Env, key: &K, val: &V)
where
    K: StorageKey + IntoVal<Env, Val>,
    V: IntoVal<Env, Val>,
{
    let s = env.storage();
    match key.durability() {
        Durability::Instance => s.instance().set(key, val),
        Durability::Persistent => s.persistent().set(key, val),
        Durability::Temporary => s.temporary().set(key, val),
    }
}

pub fn has<K>(env: &Env, key: &K) -> bool
where
    K: StorageKey + IntoVal<Env, Val>,
{
    let s = env.storage();
    match key.durability() {
        Durability::Instance => s.instance().has(key),
        Durability::Persistent => s.persistent().has(key),
        Durability::Temporary => s.temporary().has(key),
    }
}

pub fn remove<K>(env: &Env, key: &K)
where
    K: StorageKey + IntoVal<Env, Val>,
{
    let s = env.storage();
    match key.durability() {
        Durability::Instance => s.instance().remove(key),
        Durability::Persistent => s.persistent().remove(key),
        Durability::Temporary => s.temporary().remove(key),
    }
}

/// Extends the TTL of `key`'s store entry. Instance keys share one TTL, so
/// the key itself is not consulted for them.
pub fn extend_ttl<K>(env: &Env, key: &K, threshold: u32, extend_to: u32)
where
    K: StorageKey + IntoVal<Env, Val>,
{
    let s = env.storage();
    match key.durability() {
        Durability::Instance => s.instance().extend_ttl(threshold, extend_to),
        Durability::Persistent => s.persistent().extend_ttl(key, threshold, extend_to),
        Durability::Temporary => s.temporary().extend_ttl(key, threshold, extend_to),
    }
}

/// Extends the TTL of the contract instance storage.
pub fn extend_instance_ttl(env: &Env, threshold: u32, extend_to: u32) {
    env.storage().instance().extend_ttl(threshold, extend_to);
}

/// Durability class for keys that are written on hot paths.
///
/// Hot-path entry points (attest, co-attest, claim, add_attester, ...) must
/// not write any shared instance key: under Soroban's parallel execution
/// model, transactions that write the same entry cannot run in parallel, and
/// every instance write rewrites the whole instance entry. Sequence counters
/// and other per-event bookkeeping therefore live in persistent storage,
/// keyed per record/attester, so distinct transactions touch distinct
/// entries.
pub const HOT_PATH_DURABILITY: Durability = Durability::Persistent;

/// Durability class for rarely changing configuration (admin, roles, rate
/// limit config, ...). These are read on hot paths but only written by
/// admin entry points, so they may stay in the shared instance entry.
pub const CONFIG_DURABILITY: Durability = Durability::Instance;

/// Returns the durability class a hot-path key must use.
///
/// This is a compile-time constant today, but routing hot-path keys through
/// this helper keeps the intent explicit and lets the policy change in one
/// place.
pub fn hot_path_durability() -> Durability {
    HOT_PATH_DURABILITY
}

/// Returns the durability class a rarely changing config key must use.
pub fn config_durability() -> Durability {
    CONFIG_DURABILITY
}

/// Reads the current value of a per-record sequence counter and returns the
/// next one, without touching any shared entry.
///
/// `key` must be a per-record/per-attester key (for example
/// `DataKey::EventSeq(record_id)`), bound to [`HOT_PATH_DURABILITY`]. The
/// counter starts at 0 when absent, so the first call returns 1.
pub fn next_sequence<K>(env: &Env, key: &K) -> u64
where
    K: StorageKey + IntoVal<Env, Val>,
{
    let current: u64 = get(env, key).unwrap_or(0);
    let next = current.saturating_add(1);
    set(env, key, &next);
    next
}

/// Reads a per-record sequence counter without incrementing it.
pub fn peek_sequence<K>(env: &Env, key: &K) -> u64
where
    K: StorageKey + IntoVal<Env, Val>,
{
    get(env, key).unwrap_or(0)
}
