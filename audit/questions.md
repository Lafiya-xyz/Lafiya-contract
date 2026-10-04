# Questions for the Auditors

Beyond a general review, we would like explicit answers to these.

## Storage and upgrades

1. Is the lazy-migration fallback (`get_schema_version` defaulting to 1,
   and the pattern described in
   [storage-versioning.md](../docs/architecture/storage-versioning.md))
   safe against type confusion between v1 and v2 entries stored under the
   same `DataKey` variant?
2. `#[contracttype]` enums serialize variants by name. Is any reordering,
   renaming, or re-typing of `DataKey` or `Error` variants in the current
   code, or likely in future upgrades, dangerous for existing storage?
3. Can `upgrade` followed by a failed or skipped `migrate` leave a
   registry in a state where `attest` or `is_attester` returns a wrong
   answer rather than an error?
4. Are the TTL policies (`INSTANCE_BUMP_AMOUNT`, `INSTANCE_LIFETIME_THRESHOLD`,
   persistent bumps) sufficient to prevent archival of live
   allowlist and attestation entries under realistic traffic?

## Authorization

5. Given the unscoped multisig ([ADR 0007](../docs/adr/0007-unscoped-multisig-authorization.md)),
   is there any path by which signatures collected for one admin action
   can be replayed for a different action or contract?
6. Is there any function whose auth check happens after a state change or
   an external call, such that a failure path leaves partial state?
7. Can an attester craft a call to `attest` that affects another
   attester's attestation for the same `record_hash` in a way verifiers
   would misread?

## Cross-contract calls

8. `attest` trusts the configured registry's `is_attester`. What can an
   admin-set, malicious, or upgraded registry do beyond returning
   `true`/`false` (re-entrancy, budget exhaustion, event spoofing)?
9. Is `get_interface().contract_kind` safe to use as a wiring sanity
   check, given that it is self-reported by the callee?

## Verification semantics

10. Given the history bound (`MAX_HISTORY = 10`), can old attestations be
    evicted in a way that hides a revocation or misleads a verifier?
11. Does the LRC-1 commitment scheme ([ADR 0008](../docs/adr/0008-record-commitment-canonicalization.md))
    have canonicalization ambiguities that let two different records share
    a commitment? (Optional scope.)
