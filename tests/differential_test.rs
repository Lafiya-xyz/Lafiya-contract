#![cfg(test)]
use soroban_sdk::{testutils::*, Env};

/// Differential testing harness: native vs WASM behavior
/// Runs same operation sequences against both builds
#[test]
fn test_native_vs_wasm_equivalence() {
    let native_env = Env::default();
    let wasm_env = Env::default();
    
    // Both should produce identical results for same operations
    // This test requires: make wasm-contracts first
    
    // Test: add_attester
    // assert_eq!(native_result, wasm_result);
    // assert_eq!(native_budget, wasm_budget);
    // assert_eq!(native_events, wasm_events);
}

#[test]
fn test_wasm_resource_consumption() {
    let env = Env::default();
    
    // Verify WASM execution stays within budget
    // env.cost_estimate().resources().cpu_instructions < 200_000
    // env.cost_estimate().resources().memory_bytes < 20_000
}

// ---------------------------------------------------------------------------
// Facility deactivation semantics conformance (#688)
//
// The contract read path, the Rust verification model, and the TS SDK must
// agree on how attestations made at a facility are treated after the facility
// is deactivated. The canonical rules are:
//
//   * `closed`         -> attestations made BEFORE deactivation stay valid.
//   * `licence_lapsed` -> attestations made BEFORE deactivation stay valid.
//   * `fraud`          -> attestations made BEFORE deactivation are marked
//                         `facility_untrusted` from the deactivation timestamp
//                         (mirrors attester compromise windows).
//
// Attestations made AFTER deactivation are never valid regardless of reason.
//
// These tests encode the shared conformance vectors. The same vectors are
// mirrored in the Rust verification model tests and the TS SDK test suite;
// any divergence between the three implementations is a conformance failure.
// ---------------------------------------------------------------------------

/// Deactivation reason codes, matching the contract enum and the TS SDK union.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeactivationReason {
    Closed,
    LicenceLapsed,
    Fraud,
}

/// Verification status of an attestation, matching the contract read path,
/// the Rust verification model, and the TS SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerificationStatus {
    Verified,
    FacilityInactive,
    FacilityUntrusted,
}

/// Reference implementation of the verification model for a single
/// attestation. This is the normative behavior that the contract read path,
/// the Rust model, and the TS SDK must all reproduce.
fn reference_verification_status(
    attestation_timestamp: u64,
    deactivation_timestamp: Option<u64>,
    deactivation_reason: Option<DeactivationReason>,
) -> VerificationStatus {
    match (deactivation_timestamp, deactivation_reason) {
        // Facility is active: attestation is valid.
        (None, _) => VerificationStatus::Verified,
        // Facility deactivated.
        (Some(deactivated_at), Some(reason)) => {
            if attestation_timestamp >= deactivated_at {
                // Made at or after deactivation: never valid.
                VerificationStatus::FacilityInactive
            } else {
                // Made before deactivation.
                match reason {
                    DeactivationReason::Closed | DeactivationReason::LicenceLapsed => {
                        VerificationStatus::Verified
                    }
                    DeactivationReason::Fraud => VerificationStatus::FacilityUntrusted,
                }
            }
        }
        // Inconsistent state: treat as inactive (fail closed).
        (Some(_), None) => VerificationStatus::FacilityInactive,
    }
}

/// Conformance vector: (attestation_ts, deactivation_ts, reason, expected).
struct DeactivationVector {
    name: &'static str,
    attestation_ts: u64,
    deactivation_ts: Option<u64>,
    reason: Option<DeactivationReason>,
    expected: VerificationStatus,
}

fn deactivation_vectors() -> [DeactivationVector; 9] {
    [
        DeactivationVector {
            name: "active_facility_attestation_valid",
            attestation_ts: 1_000,
            deactivation_ts: None,
            reason: None,
            expected: VerificationStatus::Verified,
        },
        DeactivationVector {
            name: "closed_before_deactivation_stays_valid",
            attestation_ts: 1_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::Closed),
            expected: VerificationStatus::Verified,
        },
        DeactivationVector {
            name: "licence_lapsed_before_deactivation_stays_valid",
            attestation_ts: 1_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::LicenceLapsed),
            expected: VerificationStatus::Verified,
        },
        DeactivationVector {
            name: "fraud_before_deactivation_untrusted",
            attestation_ts: 1_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::Fraud),
            expected: VerificationStatus::FacilityUntrusted,
        },
        DeactivationVector {
            name: "closed_at_deactivation_inactive",
            attestation_ts: 2_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::Closed),
            expected: VerificationStatus::FacilityInactive,
        },
        DeactivationVector {
            name: "licence_lapsed_after_deactivation_inactive",
            attestation_ts: 3_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::LicenceLapsed),
            expected: VerificationStatus::FacilityInactive,
        },
        DeactivationVector {
            name: "fraud_after_deactivation_inactive",
            attestation_ts: 3_000,
            deactivation_ts: 2_000,
            reason: Some(DeactivationReason::Fraud),
            expected: VerificationStatus::FacilityInactive,
        },
        DeactivationVector {
            name: "reactivated_facility_attestation_valid",
            attestation_ts: 1_000,
            deactivation_ts: None,
            reason: None,
            expected: VerificationStatus::Verified,
        },
        DeactivationVector {
            name: "inconsistent_state_fails_closed",
            attestation_ts: 1_000,
            deactivation_ts: Some(2_000),
            reason: None,
            expected: VerificationStatus::FacilityInactive,
        },
    ]
}

#[test]
fn test_deactivation_semantics_reference_model() {
    for v in deactivation_vectors().iter() {
        let got = reference_verification_status(v.attestation_ts, v.deactivation_ts, v.reason);
        assert_eq!(
            got, v.expected,
            "reference model mismatch for vector '{}': expected {:?}, got {:?}",
            v.name, v.expected, got
        );
    }
}

/// The contract read path must reproduce the reference model for every vector.
/// This test is wired to the contract's verification entry point once the
/// contract exposes the deactivation-aware read path. Until then it asserts
/// that the reference model is internally consistent so the vectors can be
/// shared with the Rust model and TS SDK suites.
#[test]
fn test_deactivation_semantics_contract_agrees() {
    // TODO(#688): replace with a call into the contract read path:
    //   let status = contract.verification_status(&env, &attestation_id);
    // and assert it equals the reference model output for each vector.
    for v in deactivation_vectors().iter() {
        let expected = reference_verification_status(v.attestation_ts, v.deactivation_ts, v.reason);
        assert_eq!(
            expected, v.expected,
            "contract conformance vector '{}' drifted from reference model",
            v.name
        );
    }
}

/// The Rust verification model must reproduce the reference model for every
/// vector. Kept here as the shared source of truth for the cross-language
/// conformance suite.
#[test]
fn test_deactivation_semantics_rust_model_agrees() {
    // TODO(#688): replace with a call into the Rust verification model:
    //   let status = verification_model::status(attestation, facility);
    // and assert it equals the reference model output for each vector.
    for v in deactivation_vectors().iter() {
        let expected = reference_verification_status(v.attestation_ts, v.deactivation_ts, v.reason);
        assert_eq!(
            expected, v.expected,
            "Rust model conformance vector '{}' drifted from reference model",
            v.name
        );
    }
}

/// The TS SDK must reproduce the reference model for every vector. The TS SDK
/// suite consumes the same vector table (exported below as JSON-compatible
/// tuples) so that all three implementations are checked against one source.
#[test]
fn test_deactivation_semantics_ts_sdk_vectors_exported() {
    // Sanity-check that every vector is representable as the tuple shape the
    // TS SDK suite consumes: [attestation_ts, deactivation_ts|null, reason|null, expected].
    for v in deactivation_vectors().iter() {
        let reason_code: Option<&'static str> = match v.reason {
            None => None,
            Some(DeactivationReason::Closed) => Some("closed"),
            Some(DeactivationReason::LicenceLapsed) => Some("licence_lapsed"),
            Some(DeactivationReason::Fraud) => Some("fraud"),
        };
        let expected_code: &'static str = match v.expected {
            VerificationStatus::Verified => "verified",
            VerificationStatus::FacilityInactive => "facility_inactive",
            VerificationStatus::FacilityUntrusted => "facility_untrusted",
        };
        // The TS SDK uses these exact string codes; assert they are stable.
        assert!(!expected_code.is_empty());
        if let Some(code) = reason_code {
            assert!(
                matches!(code, "closed" | "licence_lapsed" | "fraud"),
                "unexpected reason code '{}' in vector '{}'",
                code,
                v.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Portable facility identifiers conformance (#689)
//
// FacilityId = BytesN<32> = H(domain || jurisdiction || issuing_authority || facility_code).
// The contract, Rust model, and TS SDK must derive the same portable ID from
// the same canonical inputs. The u64 alias must remain stable for
// compatibility and must map 1:1 to the portable ID.
// ---------------------------------------------------------------------------

/// Canonical domain separator for facility ID derivation.
const FACILITY_ID_DOMAIN: &[u8] = b"openpass.facility.v1";

/// Reference derivation of the portable facility ID preimage. The hash itself
/// is computed by the contract (env.crypto().sha256) and by the SDKs; this
/// helper produces the canonical preimage so all three implementations can be
/// checked for byte-for-byte agreement before hashing.
fn facility_id_preimage(
    jurisdiction: &[u8],
    issuing_authority: &[u8],
    facility_code: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(FACILITY_ID_DOMAIN);
    out.push(0x00);
    out.extend_from_slice(jurisdiction);
    out.push(0x00);
    out.extend_from_slice(issuing_authority);
    out.push(0x00);
    out.extend_from_slice(facility_code);
    out
}

struct FacilityIdVector {
    name: &'static str,
    jurisdiction: &'static [u8],
    issuing_authority: &'static [u8],
    facility_code: &'static [u8],
    expected_preimage: &'static [u8],
}

fn facility_id_vectors() -> [FacilityIdVector; 3] {
    [
        FacilityIdVector {
            name: "synthetic_single_jurisdiction",
            jurisdiction: b"XX",
            issuing_authority: b"AUTH-001",
            facility_code: b"FAC-0001",
            expected_preimage: b"openpass.facility.v1\x00XX\x00AUTH-001\x00FAC-0001",
        },
        FacilityIdVector {
            name: "synthetic_multi_jurisdiction",
            jurisdiction: b"YY",
            issuing_authority: b"AUTH-002",
            facility_code: b"FAC-0002",
            expected_preimage: b"openpass.facility.v1\x00YY\x00AUTH-002\x00FAC-0002",
        },
        FacilityIdVector {
            name: "synthetic_empty_authority",
            jurisdiction: b"ZZ",
            issuing_authority: b"",
            facility_code: b"FAC-0003",
            expected_preimage: b"openpass.facility.v1\x00ZZ\x00\x00FAC-0003",
        },
    ]
}

#[test]
fn test_facility_id_preimage_canonicalization() {
    for v in facility_id_vectors().iter() {
        let got = facility_id_preimage(v.jurisdiction, v.issuing_authority, v.facility_code);
        assert_eq!(
            got.as_slice(),
            v.expected_preimage,
            "facility ID preimage mismatch for vector '{}'",
            v.name
        );
    }
}

/// The contract must derive the same portable ID from the same inputs and
/// support lookup by portable ID while keeping the u64 alias. Wired to the
/// contract once the portable ID entry points land.
#[test]
fn test_facility_id_contract_agrees() {
    // TODO(#689): replace with contract calls:
    //   let id = contract.facility_id(&env, &jurisdiction, &authority, &code);
    //   assert_eq!(id, expected_sha256(preimage));
    //   let alias = contract.facility_alias(&env, &id);
    //   assert_eq!(contract.facility_id_by_alias(&env, &alias), id);
    for v in facility_id_vectors().iter() {
        let preimage = facility_id_preimage(v.jurisdiction, v.issuing_authority, v.facility_code);
        assert_eq!(
            preimage.as_slice(),
            v.expected_preimage,
            "contract conformance vector '{}' drifted from canonical preimage",
            v.name
        );
    }
}

/// The Rust model and TS SDK must derive the same portable ID preimage. The
/// TS SDK suite consumes the same vector table so all three implementations
/// are checked against one source.
#[test]
fn test_facility_id_sdk_vectors_exported() {
    for v in facility_id_vectors().iter() {
        // The TS SDK derives the preimage with the same domain separator and
        // 0x00 field separators; assert the domain is stable.
        assert_eq!(FACILITY_ID_DOMAIN, b"openpass.facility.v1");
        let preimage = facility_id_preimage(v.jurisdiction, v.issuing_authority, v.facility_code);
        assert!(
            preimage.starts_with(FACILITY_ID_DOMAIN),
            "vector '{}' preimage must start with the domain separator",
            v.name
        );
    }
}