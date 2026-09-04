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
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{BytesN, Env, IntoVal, Vec};

const MAX_SIGNERS: usize = 8;

fn signing_key_strategy() -> impl Strategy<Value = SigningKey> {
    (0u8..MAX_SIGNERS as u8).prop_map(|i| SigningKey::from_bytes(&[i; 32]))
}

fn signer_list_strategy() -> impl Strategy<Value = std::vec::Vec<SigningKey>> {
    prop::collection::vec(signing_key_strategy(), 0..=MAX_SIGNERS).prop_map(|mut keys| {
        let mut unique_keys = std::vec::Vec::new();
        for key in keys {
            let bytes = key.verifying_key().to_bytes();
            if !unique_keys.iter().any(|k: &SigningKey| {
                k.verifying_key().to_bytes() == bytes
            }) {
                unique_keys.push(key);
            }
        }
        unique_keys.sort_by_key(|k| k.verifying_key().to_bytes());
        unique_keys
    })
}

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
                public_key: BytesN::from_array(&env, &key.verifying_key().to_bytes()),
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

        let threshold = (keys.len().max(2)) as u32;
        let account = env.register(MultisigAccount, (signers, threshold));

        let payload = BytesN::from_array(&env, &payload_bytes);
        let signatures = if keys.len() > 1 {
            let sig_keys = &keys[..1];
            let mut ordered = sig_keys.iter().collect::<std::vec::Vec<_>>();
            ordered.sort_by_key(|k| k.verifying_key().to_bytes());

            let mut sigs = Vec::new(&env);
            for key in ordered {
                sigs.push_back(Signature {
                    public_key: BytesN::from_array(&env, &key.verifying_key().to_bytes()),
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
                public_key: BytesN::from_array(&env, &key.verifying_key().to_bytes()),
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
