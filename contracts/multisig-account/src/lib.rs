//! Soroban custom account contract implementing N-of-M multisig authorization.
//!
//! This contract enforces that transactions requiring approval must be signed
//! by at least a configured threshold of registered signers (N-of-M multisig).
//! It implements Soroban's `CustomAccountInterface` to integrate with the
//! protocol's authentication system, enabling use as a custom account for
//! administrative operations on the `attester-registry` and
//! `attestation-registry` contracts.
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
    /// The minimum number of signatures required to authorize a transaction.
    Threshold,
    /// Presence of this key (mapped to `()`) indicates the public key is a registered signer.
    Signer(BytesN<32>),
    /// The total number of registered signers.
    SignerCount,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signature {
    pub public_key: BytesN<32>,
    pub signature: BytesN<64>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    InvalidThreshold = 1,
    DuplicateSigner = 2,
    NotEnoughSigners = 3,
    BadSignatureOrder = 4,
    UnknownSigner = 5,
    NotInitialized = 6,
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
