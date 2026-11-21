//! Expiry policy (#669, #670), canonical-read projections (#668) and
//! conformance of on-chain state with `lafiya-verification-model` (#667).
//!
//! The conformance corpus enumerates every combination of the on-chain flags
//! the contract can express: record lifecycle (never attested, active,
//! withdrawn, revoked) x attester status (active, suspended, removed) x
//! disputed x expired. Withdrawn and revoked are mutually exclusive, as are
//! suspended and removed. Intentional divergences are documented in
//! `docs/specs/verification-model.md` section 9.
extern crate std;

use super::*;
use lafiya_verification_model as model;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{BytesN, Env};

use lafiya_test_utils::new_env;

type ArClient = attester_registry::AttesterRegistryClient<'static>;

fn setup() -> (Env, AttestationRegistryClient<'static>, ArClient, Address) {
    let env = new_env();
    let admin = Address::generate(&env);
    let registry_id = env.register(attester_registry::AttesterRegistry, (admin.clone(),));
    let registry = attester_registry::AttesterRegistryClient::new(&env, &registry_id);
    registry.grant_role(&attester_registry::Role::Registrar, &admin);
    registry.grant_role(&attester_registry::Role::Guardian, &admin);
    let contract_id = env.register(AttestationRegistry, (admin.clone(), registry_id));
    let client = AttestationRegistryClient::new(&env, &contract_id);
    client.grant_role(&Role::Guardian, &admin);
    client.grant_role(&Role::Revoker, &admin);
    (env, client, registry, admin)
}

fn hash(env: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(env, &[n; 32])
}

fn enroll(env: &Env, registry: &ArClient, admin: &Address) -> Address {
    let attester = Address::generate(env);
    registry.add_attester(admin, &attester);
    attester
}

// ───────────────────────── #670: bounded patient expiry ─────────────────────────

#[test]
fn consent_expiry_is_bounded_by_max_attestation_age() {
    let (env, client, registry, admin) = setup();
    let attester = enroll(&env, &registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 1);
    let max = env.ledger().timestamp() + client.get_max_attestation_age();

    for ok in [max - 1, max] {
        client.consent_attestation(&patient, &attester, &record_hash, &ok);
    }
    for bad in [max + 1, u64::MAX] {
        assert_eq!(
            client.try_consent_attestation(&patient, &attester, &record_hash, &bad),
            Err(Ok(Error::InvalidAttestationExpiry))
        );
    }
}

#[test]
fn consent_expiry_bound_saturates_instead_of_overflowing() {
    let (env, client, registry, admin) = setup();
    let attester = enroll(&env, &registry, &admin);
    let patient = Address::generate(&env);
    client.set_max_attestation_age(&u64::MAX);
    client.consent_attestation(&patient, &attester, &hash(&env, 2), &u64::MAX);
}

#[test]
fn attest_clamps_expiry_to_max_age_in_force_at_attest_time() {
    let (env, client, registry, admin) = setup();
    let attester = enroll(&env, &registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 3);
    let now = env.ledger().timestamp();
    let patient_expiry = now + DEFAULT_MAX_ATTESTATION_AGE;
    client.consent_attestation(&patient, &attester, &record_hash, &patient_expiry);

    client.set_max_attestation_age(&100);
    client.attest(&attester, &patient, &record_hash);

    let status = client.get_attestation_status(&record_hash).unwrap();
    assert_eq!(status.expires_at, Some(now + 100));
}

// ───────────────────────── #669: prospective max-age changes ─────────────────────────

#[test]
fn max_age_changes_do_not_expire_or_revive_existing_attestations() {
    let (env, client, registry, admin) = setup();
    let attester = enroll(&env, &registry, &admin);
    let patient = Address::generate(&env);
    let record_hash = hash(&env, 4);
    let now = env.ledger().timestamp();
    client.consent_attestation(&patient, &attester, &record_hash, &(now + 1_000));
    client.attest(&attester, &patient, &record_hash);

    // Tightening does not mass-expire.
    client.set_max_attestation_age(&0);
    env.ledger().set_timestamp(now + 500);
    assert!(client.is_verified(&record_hash));
    assert_eq!(
        client
            .get_attestation_status(&record_hash)
            .unwrap()
            .expires_at,
        Some(now + 1_000)
    );

    // Loosening does not revive.
    env.ledger().set_timestamp(now + 1_000);
    client.set_max_attestation_age(&u64::MAX);
    assert!(!client.is_verified(&record_hash));
}

#[test]
fn set_max_attestation_age_emits_change_event() {
    let (env, client, _registry, _admin) = setup();
    client.set_max_attestation_age(&42);
    // Reading the sequence runs a contract call, which resets the event log.
    let events = env.events().all();
    let seq = env.as_contract(&client.address, || {
        storage::get::<_, u64>(&env, &DataKey::EventSeq).unwrap_or(0)
    });
    let expected = MaxAttestationAgeChanged {
        old: DEFAULT_MAX_ATTESTATION_AGE,
        new: 42,
        seq,
    };
    assert_eq!(events, std::vec![expected.to_xdr(&env, &client.address)]);
}

// ───────────────────────── #667 / #668: conformance corpus ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lifecycle {
    Never,
    Active,
    Withdrawn,
    Revoked,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    lifecycle: Lifecycle,
    attester: model::AttesterStatus,
    disputed: bool,
    expired: bool,
}

fn corpus() -> std::vec::Vec<Scenario> {
    let mut out = std::vec::Vec::new();
    for lifecycle in [
        Lifecycle::Never,
        Lifecycle::Active,
        Lifecycle::Withdrawn,
        Lifecycle::Revoked,
    ] {
        for attester in [
            model::AttesterStatus::Active,
            model::AttesterStatus::Suspended,
            model::AttesterStatus::Removed,
        ] {
            for disputed in [false, true] {
                for expired in [false, true] {
                    out.push(Scenario {
                        lifecycle,
                        attester,
                        disputed,
                        expired,
                    });
                }
            }
        }
    }
    out
}

fn model_input(s: &Scenario) -> model::Input {
    model::Input {
        network_matches: true,
        attestation_read: model::ReadStatus::Ok,
        attester_status_read: model::ReadStatus::Ok,
        attestation_found: matches!(s.lifecycle, Lifecycle::Active | Lifecycle::Revoked),
        revoked: s.lifecycle == Lifecycle::Revoked,
        attester_status: s.attester,
        attested_in_compromise_window: false,
        age_exceeds_policy: s.expired,
        superseded: false,
        offline: false,
    }
}

#[test]
fn on_chain_reads_conform_to_verification_model_and_canonical_read() {
    let (env, client, registry, admin) = setup();
    let start = env.ledger().timestamp();
    let scenarios = corpus();
    assert_eq!(scenarios.len(), 48);

    // Build every scenario's state at `start`, then advance past the short
    // expiries so that `expired` scenarios are stale.
    let mut built = std::vec::Vec::new();
    for (i, s) in scenarios.iter().enumerate() {
        let record_hash = hash(&env, 100 + i as u8);
        let attester = enroll(&env, &registry, &admin);
        if s.lifecycle != Lifecycle::Never {
            let patient = Address::generate(&env);
            let expiry = start + if s.expired { 10 } else { 10_000 };
            client.consent_attestation(&patient, &attester, &record_hash, &expiry);
            client.attest(&attester, &patient, &record_hash);
        }
        match s.lifecycle {
            Lifecycle::Withdrawn => client.withdraw_attestation(&attester, &record_hash),
            Lifecycle::Revoked => {
                client.revoke_attestation(&admin, &record_hash, &Symbol::new(&env, "test"))
            }
            _ => {}
        }
        if s.disputed {
            env.as_contract(&client.address, || {
                env.storage()
                    .persistent()
                    .set(&DataKey::DisputeCount(record_hash.clone()), &1u32);
            });
        }
        match s.attester {
            model::AttesterStatus::Suspended => registry.suspend_attester(&admin, &attester),
            model::AttesterStatus::Removed => registry.remove_attester(&admin, &attester),
            model::AttesterStatus::Active => {}
        }
        built.push((*s, record_hash, attester));
    }
    env.ledger().set_timestamp(start + 100);

    for (s, record_hash, attester) in built {
        let ctx = std::format!("{s:?}");
        let state = client.get_verification_state(&record_hash);
        let verdict = model::evaluate(&model_input(&s)).verdict;

        // #667: on-chain state agrees with the normative model.
        match verdict {
            model::Verdict::NotFound => assert!(
                matches!(
                    state,
                    VerificationState::NeverAttested | VerificationState::Withdrawn
                ),
                "{ctx}"
            ),
            model::Verdict::Revoked => assert_eq!(state, VerificationState::Revoked, "{ctx}"),
            // Divergence D2: the canonical state does not evaluate expiry or
            // attester status; `is_verified` does.
            _ => assert_eq!(
                state,
                if s.disputed {
                    VerificationState::Disputed
                } else {
                    VerificationState::Verified
                },
                "{ctx}"
            ),
        }
        // Divergence D1: the model crate has no dispute overlay, and neither
        // does `is_verified`, so they must agree exactly.
        assert_eq!(
            client.is_verified(&record_hash),
            verdict == model::Verdict::Verified,
            "{ctx}"
        );

        // #668: deprecated reads are projections of the canonical read.
        let inactive = matches!(
            state,
            VerificationState::NeverAttested
                | VerificationState::Withdrawn
                | VerificationState::Revoked
        );
        let record_status = match state {
            VerificationState::NeverAttested => RecordStatus::NeverAttested,
            VerificationState::Verified | VerificationState::Disputed => RecordStatus::Verified,
            VerificationState::Withdrawn => RecordStatus::Withdrawn,
            VerificationState::Revoked => RecordStatus::Revoked,
        };
        assert_eq!(
            client.get_record_status(&record_hash),
            record_status,
            "{ctx}"
        );
        assert_eq!(
            client.get_attestation_status(&record_hash).is_none(),
            inactive,
            "{ctx}"
        );
        if inactive {
            assert!(!client.is_verified(&record_hash), "{ctx}");
            assert!(!client.is_attestation_trusted(&record_hash), "{ctx}");
        }
        let per_attester = match state {
            VerificationState::NeverAttested => AttesterAttestationStatus::NeverAttested,
            VerificationState::Verified | VerificationState::Disputed => {
                AttesterAttestationStatus::Active
            }
            VerificationState::Withdrawn => AttesterAttestationStatus::Withdrawn,
            VerificationState::Revoked => AttesterAttestationStatus::Revoked,
        };
        assert_eq!(
            client.get_attester_attestation_status(&record_hash, &attester),
            per_attester,
            "{ctx}"
        );
    }
}

#[test]
fn get_interface_lists_deprecated_reads() {
    let (env, client, _registry, _admin) = setup();
    let deprecated = client.get_interface().deprecated;
    assert_eq!(deprecated.len(), DEPRECATED_READS.len() as u32);
    for name in DEPRECATED_READS {
        assert!(deprecated.contains(Symbol::new(&env, name)));
    }
    assert!(!deprecated.contains(Symbol::new(&env, "get_verification_state")));
}
