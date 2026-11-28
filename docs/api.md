# API reference (rustdoc)

The rustdoc for this version is published alongside the site:

- [`attester_registry`](api/attester_registry/index.html)
- [`attestation_registry`](api/attestation_registry/index.html)
- [`multisig_account`](api/multisig_account/index.html)
- [`lafiya_commitment`](api/lafiya_commitment/index.html)
- [`lafiya_config`](api/lafiya_config/index.html)
- [`lafiya_rpc_resilience`](api/lafiya_rpc_resilience/index.html)

Build it locally with `cargo doc --workspace --no-deps --open`.

## Canonical verification read

`attestation_registry::get_verification_state(record_hash) -> VerificationState`
is the canonical on-chain verification read. The other reads are deprecated
projections of it (listed in `get_interface().deprecated`, see
[stability policy](stability-policy.md)) and will be removed in a future
interface version:

| Deprecated read | Projection of `get_verification_state` |
| --- | --- |
| `get_record_status` | Same state, with `Disputed` collapsed into `Verified` |
| `get_attester_attestation_status` | Per-attester view: `Verified`/`Disputed` -> `Active`, otherwise the same state |
| `get_attestation_status` | `None` exactly when the state is `NeverAttested`, `Withdrawn` or `Revoked`; otherwise adds the stored expiry |
| `is_verified` | `false` unless the state is `Verified` or `Disputed`; further requires the stored expiry not to have passed and the attester to be active |
| `is_attestation_trusted` | `false` when the state is `NeverAttested`, `Withdrawn` or `Revoked`; otherwise checks the attester trust cutoff |

`get_verification_state` does not evaluate expiry or attester status. A
verifier combines it with the attestation expiry and the attester-registry
status as described in [the verification model](specs/verification-model.md).
`conformance_test.rs` asserts every projection over the conformance corpus.

### Attestation expiry

The patient-selected expiry passed to `consent_attestation` must satisfy
`now < attestation_expires_at <= now + get_max_attestation_age()`, otherwise the
call fails with `InvalidAttestationExpiry` (code 17). At `attest` time the
stored expiry is `min(patient_expiry, attested_at + max_age)`, using the max age
in force then; it is reported as `expires_at` in `AttestationRecorded`. Later
`set_max_attestation_age` calls are prospective and emit
`MaxAttestationAgeChanged { old, new }`.
