# Known-Issues Register

Security-relevant issues that are already known, so auditors do not spend
time re-reporting them. For each, we ask auditors to **confirm or deny**
the stated status rather than file it as a new finding. Anything that
shows impact beyond what is described here is a new finding.

The `issues/` directory holds the original write-ups from the internal
pre-audit review.

| ID | Issue | Status | Confirm or deny |
|---|---|---|---|
| KI-1 | SEC-03: `MultisigAccount::__check_auth` ignores `auth_contexts`, so a threshold of signatures authorizes any invocation ([issues/09](../issues/09-multisig-ignores-auth-contexts-unscoped-authorization.md)) | **Accepted risk** — [ADR 0007](../docs/adr/0007-unscoped-multisig-authorization.md) | Is the risk limited to what ADR 0007 describes, given the multisig only administers the two registries? |
| KI-2 | ARCH-02: a removed or suspended attester's past attestations still verify on-chain ([issues/05](../issues/05-attestation-revocation-semantics-undefined.md)) | **Accepted** — [ADR 0006](../docs/adr/0006-attestation-revocation-semantics.md); verifiers must check current status and revocation | Is ADR 0006's verifier-side check sufficient, and can it be bypassed? |
| KI-3 | SEC-01: unbounded signature list in `__check_auth` could exhaust the budget ([issues/07](../issues/07-multisig-unbounded-signature-list-budget-dos.md)) | **Fixed** — signatures above the signer count are rejected (`TooManySigners`) | Is the bound sufficient for the largest supported signer set? |
| KI-4 | SEC-02: multisig never extended its TTL ([issues/08](../issues/08-multisig-account-no-ttl-extension-bricking-risk.md)) | **Fixed** — `__check_auth` extends the instance TTL | Can the account still be archived if it is idle longer than the bump amount? |
| KI-5 | ARCH-01: attester-registry never called `extend_ttl` ([issues/04](../issues/04-attester-registry-missing-ttl-extension-policy.md)) | **Fixed** — writes extend persistent and instance TTLs | Are there read-only paths through which live state can expire? |
| KI-6 | ARCH-03: attestation-registry could not repoint its attester-registry ([issues/06](../issues/06-attestation-registry-no-registry-repoint-path.md)) | **Fixed** — `set_attester_registry` (admin-only) | Unlike `initialize`, `set_attester_registry` does not check that the new address implements `is_attester`. Is that exploitable beyond an admin error (which would make every `attest` fail)? |
| KI-7 | Single admin can upgrade to arbitrary Wasm before multisig handover | **Accepted risk** — [ADR 0003](../docs/adr/0003-single-admin-initial-model.md) | — |
| KI-8 | `revoke_attestation` on an unknown hash returns `NotInitialized` instead of `AttestationNotFound` (see `revoke_attestation_for_unknown_hash_returns_not_initialized`) | **Open**, low | Does the misleading error code affect any client decision? |
| KI-9 | Several invariants have no dedicated test (marked **gap** in [invariants.md](invariants.md)) | **Open** | — |
| KI-10 | QA-01: load tests, cost benchmarks, and docs drifted from contract behavior ([issues/10](../issues/10-docs-and-load-test-drifted-from-actual-contract-behavior.md)) | **Partially fixed** — error and event docs are now checked by `make conformance` | — |

Issues CI-01, BUILD-01, and PROC-01 in `issues/` concerned the build and
review process, not contract security, and are resolved on `main`.
