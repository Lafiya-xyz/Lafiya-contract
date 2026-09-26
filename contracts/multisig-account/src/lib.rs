//! Soroban custom account contract implementing weighted, role-based multisig authorization.
//!
//! This contract enforces minimum signer, voting-weight, and role quorums.
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
    /// The ordered list of registered signers, used to safely replace the set.
    SignerSet,
    /// Per-signer weight and role used during authorization.
    SignerDetails(BytesN<32>),
    /// The current weighted and role-based quorum policy.
    Policy,
}

/// A configured signer and the voting weight/role assigned to that signer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerConfig {
    pub public_key: BytesN<32>,
    pub weight: u32,
    pub role: Option<soroban_sdk::Symbol>,
}

/// A minimum number of approvals required from one role.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRequirement {
    pub role: soroban_sdk::Symbol,
    pub minimum: u32,
}

/// Full multisig policy returned by `get_policy`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerPolicy {
    pub signers: Vec<SignerConfig>,
    pub weight_threshold: u32,
    pub minimum_signers: u32,
    pub role_requirements: Vec<RoleRequirement>,
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
    /// The supplied signatures do not meet the configured weight threshold.
    NotEnoughWeight = 8,
    /// The supplied signatures do not meet one or more role requirements.
    RoleQuorumNotMet = 9,
    /// A signer weight, role requirement, or policy threshold is invalid.
    InvalidPolicy = 10,
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

        let mut signer_configs = Vec::new(&env);
        for signer in signers.iter() {
            let key = DataKey::Signer(signer.clone());
            if env.storage().instance().has(&key) {
                panic_with_error!(&env, Error::DuplicateSigner);
            }
            env.storage().instance().set(&key, &());
            let config = SignerConfig {
                public_key: signer.clone(),
                weight: 1,
                role: None,
            };
            env.storage()
                .instance()
                .set(&DataKey::SignerDetails(signer), &config);
            signer_configs.push_back(config);
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &signers.len());
        env.storage().instance().set(&DataKey::SignerSet, &signers);
        env.storage().instance().set(
            &DataKey::Policy,
            &SignerPolicy {
                signers: signer_configs,
                weight_threshold: threshold,
                minimum_signers: threshold,
                role_requirements: Vec::new(&env),
            },
        );
    }

    /// Return the current ordered signer set.
    pub fn get_signers(env: Env) -> Result<Vec<BytesN<32>>, Error> {
        env.storage()
            .instance()
            .get(&DataKey::SignerSet)
            .ok_or(Error::NotInitialized)
    }

    /// Return the minimum number of signer approvals required.
    pub fn get_threshold(env: Env) -> Result<u32, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)
    }

    /// Return the current weighted and role-based signer policy.
    pub fn get_policy(env: Env) -> Result<SignerPolicy, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)
    }

    /// Replace the signer set with equal-weight signers and no role constraints.
    ///
    /// The account itself must authorize this call under its current policy.
    pub fn set_signers(env: Env, signers: Vec<BytesN<32>>, threshold: u32) -> Result<(), Error> {
        let mut configs = Vec::new(&env);
        for public_key in signers.iter() {
            configs.push_back(SignerConfig {
                public_key,
                weight: 1,
                role: None,
            });
        }
        let role_requirements = Vec::new(&env);
        Self::set_policy(env, configs, threshold, threshold, role_requirements)
    }

    /// Atomically replace the signer set and approval policy.
    ///
    /// `minimum_signers` and `weight_threshold` must both be met. Each role
    /// requirement also specifies a minimum number of approving signers with
    /// that role. The current policy must authorize this change.
    pub fn set_policy(
        env: Env,
        signers: Vec<SignerConfig>,
        weight_threshold: u32,
        minimum_signers: u32,
        role_requirements: Vec<RoleRequirement>,
    ) -> Result<(), Error> {
        env.current_contract_address().require_auth();
        Self::validate_policy(
            &signers,
            weight_threshold,
            minimum_signers,
            &role_requirements,
        )?;

        let old_signers: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerSet)
            .ok_or(Error::NotInitialized)?;

        for signer in old_signers.iter() {
            env.storage()
                .instance()
                .remove(&DataKey::Signer(signer.clone()));
            env.storage()
                .instance()
                .remove(&DataKey::SignerDetails(signer));
        }
        let mut public_keys = Vec::new(&env);
        for signer in signers.iter() {
            env.storage()
                .instance()
                .set(&DataKey::Signer(signer.public_key.clone()), &());
            env.storage()
                .instance()
                .set(&DataKey::SignerDetails(signer.public_key.clone()), &signer);
            public_keys.push_back(signer.public_key);
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &minimum_signers);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &signers.len());
        env.storage()
            .instance()
            .set(&DataKey::SignerSet, &public_keys);
        env.storage().instance().set(
            &DataKey::Policy,
            &SignerPolicy {
                signers,
                weight_threshold,
                minimum_signers,
                role_requirements,
            },
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    fn validate_policy(
        signers: &Vec<SignerConfig>,
        weight_threshold: u32,
        minimum_signers: u32,
        role_requirements: &Vec<RoleRequirement>,
    ) -> Result<(), Error> {
        if minimum_signers == 0 || minimum_signers > signers.len() {
            return Err(Error::InvalidThreshold);
        }

        let mut total_weight = 0u32;
        for (index, signer) in signers.iter().enumerate() {
            if signer.weight == 0 {
                return Err(Error::InvalidPolicy);
            }
            total_weight = total_weight
                .checked_add(signer.weight)
                .ok_or(Error::InvalidPolicy)?;
            for previous in signers.iter().take(index) {
                if previous.public_key == signer.public_key {
                    return Err(Error::DuplicateSigner);
                }
            }
        }

        if weight_threshold == 0 || weight_threshold > total_weight {
            return Err(Error::InvalidPolicy);
        }

        for (index, requirement) in role_requirements.iter().enumerate() {
            if requirement.minimum == 0 {
                return Err(Error::InvalidPolicy);
            }
            if role_requirements
                .iter()
                .take(index)
                .any(|previous| previous.role == requirement.role)
            {
                return Err(Error::InvalidPolicy);
            }
            let mut matching_signers = 0u32;
            for signer in signers.iter() {
                if signer.role == Some(requirement.role.clone()) {
                    matching_signers += 1;
                }
            }
            if requirement.minimum > matching_signers {
                return Err(Error::InvalidPolicy);
            }
        }
        Ok(())
    }
}

#[contractimpl(contracttrait)]
impl CustomAccountInterface for MultisigAccount {
    type Signature = Vec<Signature>;
    type Error = Error;

    /// Verify the authorization of a transaction using the configured quorum policy.
    ///
    /// Signatures must meet the minimum signer count and weight threshold, satisfy every role
    /// minimum, and be ordered by ascending public key.
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
        let policy: SignerPolicy = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        if signatures.len() < policy.minimum_signers {
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

        let mut signer_configs = Vec::new(&env);
        let mut signed_weight = 0u32;
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

            let signer_config: SignerConfig = env
                .storage()
                .instance()
                .get(&DataKey::SignerDetails(signature.public_key.clone()))
                .ok_or(Error::UnknownSigner)?;
            signed_weight = signed_weight
                .checked_add(signer_config.weight)
                .ok_or(Error::InvalidPolicy)?;
            signer_configs.push_back(signer_config);

            env.crypto().ed25519_verify(
                &signature.public_key,
                &signature_payload.clone().into(),
                &signature.signature,
            );
        }

        if signed_weight < policy.weight_threshold {
            return Err(Error::NotEnoughWeight);
        }

        for requirement in policy.role_requirements.iter() {
            let mut matching_signers = 0u32;
            for signer in signer_configs.iter() {
                if signer.role == Some(requirement.role.clone()) {
                    matching_signers += 1;
                }
            }
            if matching_signers < requirement.minimum {
                return Err(Error::RoleQuorumNotMet);
            }
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
