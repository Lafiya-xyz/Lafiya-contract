#![no_std]

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractimpl, contracttype,
    crypto::Hash,
    panic_with_error, BytesN, Env, Vec,
};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Threshold,
    Signer(BytesN<32>),
    SignerCount,
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
        }

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }
}

#[cfg(test)]
mod integration_test;
#[cfg(test)]
mod test;
#[cfg(test)]
mod fuzz_test;
