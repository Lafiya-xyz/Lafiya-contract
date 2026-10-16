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
