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

/// Shared bounded validation for `String` / `Bytes` inputs.
///
/// Every contract that accepts free-form text or binary blobs routes it
/// through these helpers so that persistent entry sizes (and therefore rent
/// and read fees) stay bounded and so that no arbitrary text can be stored
/// on-chain. All checks are allocation-free, `no_std` friendly and never
/// panic: failures are reported as [`ValidationError`].
pub mod validate {
    use soroban_sdk::{Bytes, Env, String};

    /// Typed validation failure. Contracts map these onto their own error
    /// enums (typically `Error::InvalidInput`).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ValidationError {
        /// Input exceeds the configured maximum length.
        TooLong,
        /// Input contains a byte outside the allowed character set.
        InvalidCharset,
        /// Input is empty where a non-empty value is required.
        Empty,
    }

    /// Maximum length of a region code, e.g. `"NG-LA"`.
    pub const REGION_CODE_MAX: u32 = 8;

    /// Returns `true` for ASCII letters (`A-Z`, `a-z`).
    #[inline]
    pub const fn is_ascii_alpha(b: u8) -> bool {
        (b >= b'A' && b <= b'Z') || (b >= b'a' && b <= b'z')
    }

    /// Returns `true` for ASCII digits (`0-9`).
    #[inline]
    pub const fn is_ascii_digit(b: u8) -> bool {
        b >= b'0' && b <= b'9'
    }

    /// Returns `true` for printable ASCII (space through `~`).
    #[inline]
    pub const fn is_printable_ascii(b: u8) -> bool {
        b >= 0x20 && b <= 0x7e
    }

    /// Validates that `s` is at most `max` bytes and contains only printable
    /// ASCII. Empty strings are allowed; use [`bounded_ascii_nonempty`] when a
    /// value is required.
    pub fn bounded_ascii(s: &String, max: u32) -> Result<(), ValidationError> {
        let len = s.len();
        if len > max {
            return Err(ValidationError::TooLong);
        }
        let mut buf = [0u8; 64];
        let mut offset = 0u32;
        while offset < len {
            let chunk = if len - offset > buf.len() as u32 {
                buf.len() as u32
            } else {
                len - offset
            };
            s.copy_into_slice(&mut buf[..chunk as usize]);
            let mut i = 0usize;
            while i < chunk as usize {
                if !is_printable_ascii(buf[i]) {
                    return Err(ValidationError::InvalidCharset);
                }
                i += 1;
            }
            offset += chunk;
        }
        Ok(())
    }

    /// Like [`bounded_ascii`] but rejects empty input.
    pub fn bounded_ascii_nonempty(s: &String, max: u32) -> Result<(), ValidationError> {
        if s.is_empty() {
            return Err(ValidationError::Empty);
        }
        bounded_ascii(s, max)
    }

    /// Validates that `b` is at most `max` bytes. No charset restriction is
    /// applied; use this for opaque binary payloads such as hashes.
    pub fn bounded_bytes(b: &Bytes, max: u32) -> Result<(), ValidationError> {
        if b.len() > max {
            return Err(ValidationError::TooLong);
        }
        Ok(())
    }

    /// Validates a region code: 2..=`REGION_CODE_MAX` bytes, ASCII
    /// alphanumeric with `-` separators, and never starting or ending with a
    /// separator. Example valid values: `"NG"`, `"NG-LA"`, `"US-CA-01"`.
    pub fn region_code(s: &String) -> Result<(), ValidationError> {
        let len = s.len();
        if len == 0 {
            return Err(ValidationError::Empty);
        }
        if len > REGION_CODE_MAX {
            return Err(ValidationError::TooLong);
        }
        let mut buf = [0u8; REGION_CODE_MAX as usize];
        s.copy_into_slice(&mut buf[..len as usize]);
        let mut i = 0usize;
        while i < len as usize {
            let b = buf[i];
            let ok = is_ascii_alpha(b) || is_ascii_digit(b) || b == b'-';
            if !ok {
                return Err(ValidationError::InvalidCharset);
            }
            if b == b'-' && (i == 0 || i == len as usize - 1) {
                return Err(ValidationError::InvalidCharset);
            }
            i += 1;
        }
        Ok(())
    }

    /// Convenience wrapper that builds a `String` from a byte slice for tests
    /// and callers that already hold raw bytes.
    pub fn string_from_bytes(env: &Env, bytes: &[u8]) -> String {
        let mut out = Bytes::new(env);
        let mut i = 0usize;
        while i < bytes.len() {
            out.push_back(bytes[i]);
            i += 1;
        }
        String::from_bytes(env, &out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn s(env: &Env, v: &str) -> String {
            string_from_bytes(env, v.as_bytes())
        }

        #[test]
        fn bounded_ascii_accepts_within_limit() {
            let env = Env::default();
            assert_eq!(bounded_ascii(&s(&env, "Lagos"), 16), Ok(()));
            assert_eq!(bounded_ascii(&s(&env, ""), 16), Ok(()));
        }

        #[test]
        fn bounded_ascii_rejects_oversized() {
            let env = Env::default();
            assert_eq!(
                bounded_ascii(&s(&env, "0123456789"), 4),
                Err(ValidationError::TooLong)
            );
        }

        #[test]
        fn bounded_ascii_rejects_non_printable() {
            let env = Env::default();
            assert_eq!(
                bounded_ascii(&s(&env, "bad\u{7f}"), 16),
                Err(ValidationError::InvalidCharset)
            );
            assert_eq!(
                bounded_ascii(&s(&env, "tab\there"), 16),
                Err(ValidationError::InvalidCharset)
            );
        }

        #[test]
        fn bounded_ascii_nonempty_rejects_empty() {
            let env = Env::default();
            assert_eq!(
                bounded_ascii_nonempty(&s(&env, ""), 16),
                Err(ValidationError::Empty)
            );
            assert_eq!(bounded_ascii_nonempty(&s(&env, "ok"), 16), Ok(()));
        }

        #[test]
        fn bounded_bytes_enforces_limit() {
            let env = Env::default();
            let mut b = Bytes::new(&env);
            b.push_back(1);
            b.push_back(2);
            assert_eq!(bounded_bytes(&b, 2), Ok(()));
            assert_eq!(bounded_bytes(&b, 1), Err(ValidationError::TooLong));
        }

        #[test]
        fn region_code_accepts_valid() {
            let env = Env::default();
            assert_eq!(region_code(&s(&env, "NG")), Ok(()));
            assert_eq!(region_code(&s(&env, "NG-LA")), Ok(()));
            assert_eq!(region_code(&s(&env, "US-CA-01")), Ok(()));
        }

        #[test]
        fn region_code_rejects_invalid() {
            let env = Env::default();
            assert_eq!(region_code(&s(&env, "")), Err(ValidationError::Empty));
            assert_eq!(
                region_code(&s(&env, "NG-LAGOS-ISLAND")),
                Err(ValidationError::TooLong)
            );
            assert_eq!(
                region_code(&s(&env, "NG_LA")),
                Err(ValidationError::InvalidCharset)
            );
            assert_eq!(
                region_code(&s(&env, "-NG")),
                Err(ValidationError::InvalidCharset)
            );
            assert_eq!(
                region_code(&s(&env, "NG-")),
                Err(ValidationError::InvalidCharset)
            );
        }
    }
}
