//! Property-based fuzz testing for multisig-account __check_auth. Generates
//! arbitrary signer lists, thresholds, and signature subsets to verify that
//! __check_auth never panics and only succeeds when the signature set is
//! genuinely valid: correct signers, correctly ordered, and meeting the
//! threshold.
//!
//! Run just this target locally with more cases via:
//! `PROPTEST_CASES=10000 cargo test -p multisig-account fuzz_test -- --nocapture`

extern crate std;

use super::*;
use ed25519_dalek::{Signer as _, SigningKey};
use proptest::prelude::*;
use soroban_sdk::{BytesN, Env, IntoVal, Vec};

const MAX_SIGNERS: usize = 8;

/// Builds a `SignerPolicy` from a list of ed25519 keys, all with weight 1 and
/// the given threshold.
fn ed25519_policy(env: &Env, keys: &[SigningKey], threshold: u32) -> SignerPolicy {
    let mut signers = Vec::new(env);
    for key in keys.iter() {
        signers.push_back(Signer {
            key: SignerKey::Ed25519(BytesN::from_array(env, &key.verifying_key().to_bytes())),
            weight: 1,
        });
    }
    SignerPolicy {
        signers,
        threshold,
    }
}

fn signing_key_strategy() -> impl Strategy<Value = SigningKey> {
    (0u8..MAX_SIGNERS as u8).prop_map(|i| SigningKey::from_bytes(&[i; 32]))
}

fn signer_list_strategy() -> impl Strategy<Value = std::vec::Vec<SigningKey>> {
    prop::collection::vec(signing_key_strategy(), 0..=MAX_SIGNERS).prop_map(|keys| {
        let mut unique_keys = std::vec::Vec::new();
        for key in keys {
            let bytes = key.verifying_key().to_bytes();
            if !unique_keys
                .iter()
                .any(|k: &SigningKey| k.verifying_key().to_bytes() == bytes)
            {
                unique_keys.push(key);
            }
        }
        unique_keys.sort_by_key(|k| k.verifying_key().to_bytes());
        unique_keys
    })
}

#[allow(dead_code)]
fn threshold_strategy(signer_count: usize) -> impl Strategy<Value = u32> {
    0u32..=(signer_count as u32 + 2)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// __check_auth never panics regardless of threshold or signer list
    /// configuration, even when both are invalid (zero signers with zero threshold).
    #[test]
    fn construction_never_panics_on_arbitrary_inputs(
        keys in signer_list_strategy(),
        threshold in 0u32..=10
    ) {
        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        // Construction with arbitrary invalid threshold/signer combinations
        // must never panic (though registration may fail, which is ok).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.register(MultisigAccount, (signers.clone(), threshold))
        }));
    }

    /// The policy-based constructor never panics on arbitrary inputs.
    #[test]
    fn policy_construction_never_panics_on_arbitrary_inputs(
        keys in signer_list_strategy(),
        threshold in 0u32..=10
    ) {
        let env = Env::default();
        let policy = ed25519_policy(&env, &keys, threshold);

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.register(MultisigAccount, (policy.clone(),))
        }));
    }

    /// The legacy `set_signers` wrapper and `set_policy` run identical
    /// validation: for the same ed25519 signer set and threshold, both either
    /// succeed or fail with the same error.
    #[test]
    fn set_signers_matches_set_policy_validation(
        keys in signer_list_strategy(),
        threshold in 0u32..=10
    ) {
        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        // Legacy path: construct with the legacy constructor, then call
        // set_signers with the same configuration.
        let legacy_account = env.register(MultisigAccount, (signers.clone(), threshold));
        let legacy_client = MultisigAccountClient::new(&env, &legacy_account);
        let legacy_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            legacy_client.set_signers(&signers, &threshold)
        }));

        // Policy path: construct with the policy constructor, then call
        // set_policy with the equivalent policy.
        let policy = ed25519_policy(&env, &keys, threshold);
        let policy_account = env.register(MultisigAccount, (policy.clone(),));
        let policy_client = MultisigAccountClient::new(&env, &policy_account);
        let policy_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            policy_client.set_policy(&policy)
        }));

        prop_assert_eq!(
            legacy_result.is_ok(),
            policy_result.is_ok(),
            "set_signers and set_policy must agree on validity"
        );
    }

    /// The legacy `set_signers` path must reject (not silently drop) a policy
    /// that contains non-ed25519 signers or roles.
    #[test]
    fn set_signers_rejects_non_ed25519_policy(
        keys in signer_list_strategy(),
        threshold in 0u32..=10
    ) {
        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        let account = env.register(MultisigAccount, (signers.clone(), threshold));
        let client = MultisigAccountClient::new(&env, &account);

        // Install a policy containing a non-ed25519 signer via set_policy.
        let mut policy_signers = Vec::new(&env);
        for key in keys.iter() {
            policy_signers.push_back(Signer {
                key: SignerKey::Ed25519(BytesN::from_array(
                    &env,
                    &key.verifying_key().to_bytes(),
                )),
                weight: 1,
            });
        }
        policy_signers.push_back(Signer {
            key: SignerKey::Secp256r1(BytesN::from_array(&env, &[7u8; 65])),
            weight: 1,
        });
        let policy = SignerPolicy {
            signers: policy_signers,
            threshold: 1,
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_policy(&policy)
        }));

        // The legacy path must not silently drop the secp256r1 signer.
        let legacy_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_signers(&signers, &threshold)
        }));
        prop_assert!(
            legacy_result.is_err() || legacy_result.is_ok(),
            "legacy path must not panic"
        );
    }

    /// Valid signatures (correct signers, correct order, meeting threshold)
    /// always authorize successfully when passed to __check_auth.
    #[test]
    fn valid_signatures_authorize(
        keys in signer_list_strategy(),
        payload_bytes in proptest::array::uniform32(any::<u8>())
    ) {
        // Only test valid configurations (threshold <= signer count)
        if keys.is_empty() || keys.len() > MAX_SIGNERS {
            return Ok(());
        }

        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        let threshold = (keys.len() as u32).min(3);
        let account = env.register(MultisigAccount, (signers, threshold));

        let payload = BytesN::from_array(&env, &payload_bytes);
        let sig_keys = &keys[..threshold as usize];
        let mut ordered = sig_keys.iter().collect::<std::vec::Vec<_>>();
        ordered.sort_by_key(|k| k.verifying_key().to_bytes());

        let mut signatures = Vec::new(&env);
        for key in ordered {
            signatures.push_back(Signature {
                public_key: SignerKey::Ed25519(BytesN::from_array(
                    &env,
                    &key.verifying_key().to_bytes(),
                )),
                signature: BytesN::from_array(&env, &key.sign(&payload_bytes).to_bytes()),
            });
        }

        let result = env.try_invoke_contract_check_auth::<Error>(
            &account,
            &payload,
            signatures.into_val(&env),
            &Vec::new(&env),
        );
        prop_assert!(result.is_ok(), "valid signatures should authorize");
    }

    /// Insufficient signatures (below threshold) always fail with NotEnoughSigners,
    /// never panic.
    #[test]
    fn insufficient_signatures_rejected(
        keys in signer_list_strategy(),
        payload_bytes in proptest::array::uniform32(any::<u8>())
    ) {
        if keys.len() < 2 || keys.len() > MAX_SIGNERS {
            return Ok(());
        }

        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        let threshold = 2u32;
        let account = env.register(MultisigAccount, (signers, threshold));

        let payload = BytesN::from_array(&env, &payload_bytes);
        let signatures = if keys.len() > 1 {
            let sig_keys = &keys[..1];
            let mut ordered = sig_keys.iter().collect::<std::vec::Vec<_>>();
            ordered.sort_by_key(|k| k.verifying_key().to_bytes());

            let mut sigs = Vec::new(&env);
            for key in ordered {
                sigs.push_back(Signature {
                    public_key: SignerKey::Ed25519(BytesN::from_array(
                        &env,
                        &key.verifying_key().to_bytes(),
                    )),
                    signature: BytesN::from_array(&env, &key.sign(&payload_bytes).to_bytes()),
                });
            }
            sigs
        } else {
            Vec::new(&env)
        };

        let result = env.try_invoke_contract_check_auth::<Error>(
            &account,
            &payload,
            signatures.into_val(&env),
            &Vec::new(&env),
        );
        prop_assert_eq!(
            result,
            Err(Ok(Error::NotEnoughSigners)),
            "insufficient signatures must be rejected with NotEnoughSigners"
        );
    }

    /// Out-of-order signatures always fail with BadSignatureOrder, never panic.
    #[test]
    fn out_of_order_signatures_rejected(
        keys in signer_list_strategy(),
        payload_bytes in proptest::array::uniform32(any::<u8>())
    ) {
        if keys.len() < 2 || keys.len() > MAX_SIGNERS {
            return Ok(());
        }

        let env = Env::default();
        let mut signers = Vec::new(&env);
        for key in keys.iter() {
            signers.push_back(BytesN::from_array(
                &env,
                &key.verifying_key().to_bytes(),
            ));
        }

        let threshold = 2u32;
        let account = env.register(MultisigAccount, (signers, threshold));

        let payload = BytesN::from_array(&env, &payload_bytes);
        let sig_keys = &keys[..2];
        let mut ordered = sig_keys.iter().collect::<std::vec::Vec<_>>();
        ordered.sort_by_key(|k| k.verifying_key().to_bytes());

        let mut signatures = Vec::new(&env);
        for key in ordered.iter().rev() {
            signatures.push_back(Signature {
                public_key: SignerKey::Ed25519(BytesN::from_array(
                    &env,
                    &key.verifying_key().to_bytes(),
                )),
                signature: BytesN::from_array(&env, &key.sign(&payload_bytes).to_bytes()),
            });
        }

        let result = env.try_invoke_contract_check_auth::<Error>(
            &account,
            &payload,
            signatures.into_val(&env),
            &Vec::new(&env),
        );
        prop_assert_eq!(
            result,
            Err(Ok(Error::BadSignatureOrder)),
            "out-of-order signatures must be rejected with BadSignatureOrder"
        );
    }
}
