# Glossary

Canonical definitions of the terms used across this repository's `README.md`, ADRs,
runbooks, and architecture docs. If a document needs to explain one of these terms, link
here instead of redefining it, so there is a single source of truth for new contributors.

Terms in *italics* within a definition are themselves defined in this glossary.

## Terms

### Admin

The address authorized to perform administrative operations on a registry: `initialize`,
attester add/remove/suspend/reinstate, pause/unpause, `upgrade`, and `migrate`. Pre-alpha
contracts start with a single admin address ([ADR-0003](adr/0003-single-admin-initial-model.md));
the intended shape is a *multisig account* ([ADR-0007](adr/0007-unscoped-multisig-authorization.md)).

### Allowlist

The set of addresses authorized to submit *attestations*, maintained by the *attester
registry*. "Allowlisted" means currently authorized: added via `add_attester` and not
removed or *suspended*.

### Attester

An on-chain Stellar address that has been added to the *attester registry*'s *allowlist*
and is therefore authorized to submit *attestations*. In the real-world model the attester
is a licensed health worker — typically a *CHW* — who verified the patient's emergency
record before it was attested. `attest()` requires the attester's authorization and checks
`is_attester` against the allowlist on every write.

### Attester registry (`attester-registry`)

The Soroban contract that maintains the *allowlist*: adding, removing, suspending, and
reinstating *attesters*, plus pausing/unpausing and contract upgrades. It is the single
source of truth for who may attest. See `contracts/attester-registry/src/lib.rs`.

### Attestation

The on-chain record written by the *attestation registry* when an *attester* verifies a
record: `{ attester: Address, timestamp: u64 }`, stored keyed by the *record hash* with a
bounded per-hash history (10 entries). Written by `attest()`, removed by
`revoke_attestation()`. See [ADR-0006](adr/0006-attestation-revocation-semantics.md).

### Attestation registry (`attestation-registry`)

The Soroban contract that stores *attestations* keyed by *record hash*. It never reads or
interprets the hash (see *record commitment*), and checks the *attester registry*'s
*allowlist* on every write. See `contracts/attestation-registry/src/lib.rs`.

### CHW (Community Health Worker)

The human actor who registers patients and verifies their emergency health records —
typically the last-mile health worker in the Nigerian context this project targets. On-chain
a CHW is represented by an *attester* address. CHWs are the intended recipients of USDC
micro-payments per verified registration under the incentive layer
([ADR-0009](adr/0009-treasury-asset-custody-model.md)).

### LRC-1 (Lafiya Record Commitment v1)

The canonical construction for *record commitments*, defined by
[ADR-0008](adr/0008-record-commitment-canonicalization.md):
`SHA-256("lafiya:record-commitment" || 0x01 || canonical_payload)`. Reference
implementations live in `crates/lafiya-commitment`.

### Multisig account (`multisig-account`)

A reusable N-of-M Soroban account contract: its `__check_auth` verifies ordered, unique
ed25519 signatures from the configured signer set whenever another contract calls
`require_auth()` on it. It secures registry *admin* authorization. See
[ADR-0007](adr/0007-unscoped-multisig-authorization.md).

### Record

The off-chain emergency health record — the handful of facts that change treatment (blood
group, genotype, allergies, current medications, chronic conditions) — held in
`lafiya-web`'s encrypted, access-controlled database. Only a *record commitment* touches
the chain.

### Record commitment (a.k.a. record hash)

A 32-byte opaque value (`BytesN<32>`, on-chain parameter name `record_hash`) that binds an
*attestation* to a specific off-chain *record* without revealing it. The contracts treat it
as an opaque identifier only — they never compute, read, or interpret it
([ADR-0001](adr/0001-hash-only-on-chain-footprint.md)). "Record hash" is the informal term
and the contract parameter name; "record commitment" is the canonical term, with the
byte-exact construction defined by *LRC-1* ([ADR-0008](adr/0008-record-commitment-canonicalization.md)).
Hashes recorded before LRC-1 are treated as legacy/unversioned (version `0x00`).

### Schema version

The version number of a contract's on-chain storage schema. Every contract carries
`const SCHEMA_VERSION: u32` (currently `1` for both registries), writes it to instance
storage during `initialize()`, and exposes it via `get_schema_version()`. `0` means no
version recorded (legacy pre-versioning deployment or uninitialized contract). Bumping
`SCHEMA_VERSION` signals a schema-changing upgrade that requires `migrate()` to run the
ordered migration steps. See [docs/architecture/storage-versioning.md](architecture/storage-versioning.md)
and [docs/runbooks/contract-upgrade.md](runbooks/contract-upgrade.md).

### Suspended

An *attester* that remains on the *allowlist* but is temporarily blocked from attesting
(`suspend_attester` / `reinstate_attester`). Suspension is distinct from removal.

### USDC incentive pool

The M2 design for paying *CHWs*: grant and donor funds flow on-chain into a pool from which
CHWs receive USDC micro-payments per verified registration. The treasury and custody model
is defined by [ADR-0009](adr/0009-treasury-asset-custody-model.md). No payout contract is
implemented yet.
