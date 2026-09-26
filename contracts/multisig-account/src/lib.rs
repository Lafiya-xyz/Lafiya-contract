//! Soroban custom account contract implementing N-of-M multisig authorization.
//!
//! This contract enforces that transactions requiring approval must be signed
//! by at least a configured threshold of registered signers (N-of-M multisig).
//! It implements Soroban's `CustomAccountInterface` to integrate with the
//! protocol's authentication system, enabling use as a custom account for
//! administrative operations on the `attester-registry` and
//! `attestation-registry` contracts.
#![no_std]

// Unlike `attester-registry` and `attestation-registry`, this crate does not set
// `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. `__constructor`
// cannot return a `Result` (Soroban constructors return `()`), so it deliberately uses
// `panic_with_error!` to reject invalid construction parameters with a contract error
// code, which the blanket deny would flag despite being the correct pattern here.

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    panic_with_error, Bytes, BytesN, Env, Vec,
};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// The minimum number of signatures required to authorize a transaction.
    Threshold,
    /// Presence of this key (mapped to `()`) indicates the public key is a registered signer.
    Signer(BytesN<32>),
    /// The total number of registered signers.
    SignerCount,
    /// The configured signer public keys.
    Signers,
}

/// A single ed25519 signature from one signer in the multisig set.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signature {
    /// The public key of the signer who created this signature.
    pub public_key: BytesN<32>,
    /// The ed25519 signature bytes.
    pub signature: BytesN<64>,
}

/// Errors returned by the multisig-account contract's public entry points.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The configured threshold is zero or exceeds the signer count.
    InvalidThreshold = 1,
    /// The signer configuration contains duplicate public keys.
    DuplicateSigner = 2,
    /// The supplied signature count is below the configured threshold.
    NotEnoughSigners = 3,
    /// Signatures are not strictly ordered by ascending public key.
    BadSignatureOrder = 4,
    /// A signature corresponds to a public key that is not a configured signer.
    UnknownSigner = 5,
    /// The contract has not been initialized; threshold or signer count is unavailable.
    NotInitialized = 6,
    /// The supplied signature count exceeds the configured signer count.
    TooManySigners = 7,
}

/// Instance storage TTL policy:
/// - Threshold: 30 days (17280 * 30 = 518400 ledgers)
/// - Extend to: 90 days (17280 * 90 = 1555200 ledgers)
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

#[contractevent]
#[derive(Clone, Debug)]
pub struct MultisigConfigured {
    #[topic]
    pub threshold: u32,
    pub signers: Vec<BytesN<32>>,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AuthorizationApproved {
    #[topic]
    pub timestamp: u64,
    pub payload: Bytes,
    pub signers: Vec<BytesN<32>>,
}

#[contract]
pub struct MultisigAccount;

#[contractimpl]
impl MultisigAccount {
    /// Initialize the multisig account with a set of authorized signers and a signature threshold.
    ///
    /// # Arguments
    /// * `signers` — A vector of ed25519 public keys (32 bytes each) authorized to sign transactions.
    /// * `threshold` — The minimum number of signatures required to authorize a transaction; must be > 0 and ≤ the signer count.
    pub fn __constructor(env: Env, signers: Vec<BytesN<32>>, threshold: u32) {
        if threshold == 0 || threshold > signers.len() {
            panic_with_error!(&env, Error::InvalidThreshold);
        }

        for signer in signers.iter() {
            let key = DataKey::Signer(signer);
            if env.storage().instance().has(&key) {
                panic_with_error!(&env, Error::DuplicateSigner);
            }
            env.storage().instance().set(&key, &());
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &signers.len());
        env.storage().instance().set(&DataKey::Signers, &signers);

        MultisigConfigured {
            threshold,
            signers: Self::sorted_signers(&env, &signers),
        }
        .publish(&env);
    }

    /// Return the configured threshold.
    pub fn get_threshold(env: Env) -> Result<u32, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)
    }

    /// Return all configured signers in the canonical ascending-key order
    /// required by `__check_auth`.
    pub fn get_signers(env: Env) -> Result<Vec<BytesN<32>>, Error> {
        let signers: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::Signers)
            .ok_or(Error::NotInitialized)?;
        Ok(Self::sorted_signers(&env, &signers))
    }

    fn sorted_signers(env: &Env, signers: &Vec<BytesN<32>>) -> Vec<BytesN<32>> {
        let mut sorted = Vec::new(env);
        for signer in signers.iter() {
            let mut next = Vec::new(env);
            let mut inserted = false;
            for current in sorted.iter() {
                if !inserted && signer < current {
                    next.push_back(signer.clone());
                    inserted = true;
                }
                next.push_back(current);
            }
            if !inserted {
                next.push_back(signer);
            }
            sorted = next;
        }
        sorted
    }
}

#[contractimpl(contracttrait)]
impl CustomAccountInterface for MultisigAccount {
    type Signature = Vec<Signature>;
    type Error = Error;

    /// Verify the authorization of a transaction by checking N-of-M ed25519 signatures.
    ///
    /// Verifies that the supplied signatures meet the configured threshold and each belongs to
    /// an authorized signer, with signatures ordered in ascending public-key order.
    ///
    /// # Arguments
    /// * `signature_payload` — A 32-byte hash of the transaction to authorize.
    /// * `signatures` — A vector of ed25519 signatures, each with a public key and signature bytes, ordered by ascending public key.
    /// * `_auth_contexts` — Intentionally unused; see [ADR-0007](../adr/0007-unscoped-multisig-authorization.md) for why this account does not scope authorization to specific contracts or functions during pre-alpha.
    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signatures: Self::Signature,
        _auth_contexts: Vec<Context>,
    ) -> Result<(), Error> {
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)?;

        if signatures.len() < threshold {
            return Err(Error::NotEnoughSigners);
        }

        let signer_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCount)
            .ok_or(Error::NotInitialized)?;

        if signatures.len() > signer_count {
            return Err(Error::TooManySigners);
        }

        let mut authorized_signers = Vec::new(&env);
        for index in 0..signatures.len() {
            let signature = signatures.get_unchecked(index);
            if index > 0 {
                let previous = signatures.get_unchecked(index - 1);
                if previous.public_key >= signature.public_key {
                    return Err(Error::BadSignatureOrder);
                }
            }

            if !env
                .storage()
                .instance()
                .has(&DataKey::Signer(signature.public_key.clone()))
            {
                return Err(Error::UnknownSigner);
            }

            env.crypto().ed25519_verify(
                &signature.public_key,
                &signature_payload.clone().into(),
                &signature.signature,
            );
            authorized_signers.push_back(signature.public_key);
        }

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        AuthorizationApproved {
            timestamp: env.ledger().timestamp(),
            payload: signature_payload.clone().into(),
            signers: authorized_signers,
        }
        .publish(&env);

        Ok(())
    }
}

#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod integration_test;
#[cfg(test)]
mod test;
