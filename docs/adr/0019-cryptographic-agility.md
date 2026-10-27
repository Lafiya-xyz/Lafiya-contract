# ADR-0019: Cryptographic agility, algorithm identifiers and post-quantum readiness

- **Status:** Proposed
- **Date:** 2026-10-17
- **Deciders:** Lafiya contract maintainers

> **Terminology:** shared terms (*attester*, *record commitment*, *schema version*, …) are
> defined canonically in the [glossary](../glossary.md). Link there rather than redefining them.

## Context

Commitments, attestations, QR payloads and SDKs all assume SHA-256 and Ed25519, mostly
implicitly. An emergency card must stay verifiable for at least 10 years
([stability policy](../stability-policy.md)). Over that span a hash may weaken, a signature
scheme may be broken (most plausibly Ed25519 and ECDSA by a cryptographically relevant
quantum computer, a "CRQC"), or the protocol may want new primitives for zero-knowledge or
cost reasons. We need to know where the assumptions live, how each one is identified, and how
to migrate without orphaning historical attestations. The v1 wire formats are not frozen yet,
so now is the cheapest time to add missing algorithm fields.

## Inventory of primitives

| # | Where | Primitive | Algorithm identified by | Security role |
| --- | --- | --- | --- | --- |
| 1 | LRC-1 commitment (`crates/lafiya-commitment`, `packages/verifier-sdk/src/commitment.ts`, TS/Python references) | SHA-256 over `DOMAIN_TAG ‖ VERSION_V1 ‖ payload` | Version byte `0x01` in the preimage. QR `lrc_version` | Binding and hiding (with salt) |
| 2 | LRC-2 leaves and root (`lrc2.rs`, ADR-0012) | SHA-256, domain-tagged | QR `lrc_version = 2` | Binding and hiding |
| 3 | Salt derivation (ADR-0018, `salt.rs`) | HKDF-SHA256 | `info` layout byte `0x01`, extract salt `…:v1` | Salt secrecy |
| 4 | Merkle batch anchoring (`anchor_batch`, [merkle-batch-attestations.md](../merkle-batch-attestations.md)) | SHA-256, `0x00`/`0x01` leaf and node prefixes | **Nothing** | Binding |
| 5 | On-chain `record_hash` / `root` (`attestation-registry`) | Opaque `BytesN<32>`; the contract never hashes | Implied by the card's `lrc_version` | Lookup key |
| 6 | Attester authorization (`require_auth`) | Ed25519 (Stellar `G…` accounts) or the account contract's own scheme | Stellar address type / account contract | Attestation integrity |
| 7 | `multisig-account` `__check_auth` | Ed25519 and secp256r1 via host functions | `SignerKey` enum variant | Multisig integrity |
| 8 | `packages/multisig-auth` | Ed25519 signatures over the auth payload | Implied | Multisig integrity |
| 9 | Offline receipt (ADR-0014, QR key `7`) | Ed25519 over a SHA-256 canonical preimage. The trust bundle is Ed25519-signed | Receipt `v = 1` (implied) | Offline integrity |
| 10 | Patient signature (QR key `9`) | Ed25519 over `SHA-256("lafiya:qr:patient-sig:v1" ‖ CBOR)` | Domain tag `v1` (implied) | Anti-substitution (soft) |
| 11 | QR `network_id` (key `2`) | `SHA-256(passphrase)[0:4]` | Implied | Identifier only, not security |
| 12 | W3C VC export ([ADR-0012](0012-w3c-vc-export-format.md)) | `eddsa-jcs-2022` / `eddsa-rdfc-2022` | Data Integrity `cryptosuite` property | Credential integrity |
| 13 | Stellar network itself (transaction signatures, ledger hashes) | Ed25519, SHA-256 | Stellar protocol | Root of on-chain trust |
| 14 | Tooling (`lafiya-cli` audit log and deployment ledger, `lafiya-config` record hashes, network id in `submit`) | SHA-256 | Field names (`wasm_sha256`, …) | Operator audit trail |

Rows 1–3 already carry a version that pins the algorithm. Rows 7 and 12 are agile by
construction. Row 13 is outside our control. Rows 4, 9 and 10 are the gaps, along with row 8,
which follows row 7.

## Decision

### 1. Algorithm-id registry

Algorithms are named by one-byte identifiers in three namespaces (hash, signature, KDF),
defined in `crates/lafiya-commitment/src/alg.rs`. That file and the table below must agree.

| Namespace | Id | Algorithm | Status |
| --- | --- | --- | --- |
| all | `0x00` | Implied by the enclosing format version | Active (every v1 field) |
| hash | `0x01` | SHA-256 | Active |
| hash | `0x02` | SHA3-256 | Reserved: successor candidate |
| hash | `0x03` | Keccak-256 | Reserved: Soroban host function |
| hash | `0x04` | SHA-512/256 | Reserved |
| hash | `0x10` | Poseidon over BN254 | Reserved: ZK-friendly commitments |
| sig | `0x01` | Ed25519 | Active |
| sig | `0x02` | ECDSA P-256 (secp256r1) | Active in `multisig-account` |
| sig | `0x03` | ECDSA secp256k1 | Reserved |
| sig | `0x10` | ML-DSA-44 (FIPS 204) | Reserved: post-quantum |
| sig | `0x11` | ML-DSA-65 (FIPS 204) | Reserved: post-quantum |
| sig | `0x12` | SLH-DSA-SHA2-128s (FIPS 205) | Reserved: post-quantum |
| sig | `0x13` | FN-DSA-512 (Falcon) | Reserved: pending FIPS 206 |
| sig | `0x20` | Hybrid Ed25519 + ML-DSA-44 (both must verify) | Reserved: transition |
| kdf | `0x01` | HKDF-SHA256 | Active |
| all | `0xF0`–`0xFF` | Private use; never accepted in production | — |

Registry rules:

- Ids are never reused or renumbered, and retired ids stay reserved, matching the rule for
  contract error codes.
- Adding an id is a minor change. *Activating* one (shipping it in a wire format) needs an
  ADR, shared test vectors in every implementation, and a stability-policy entry.
- An id names one exact algorithm and parameter set. Encoding details that are not
  parameters, such as domain tags and preimage layout, belong to the format version.
- An algorithm id must be bound into whatever it authenticates: in the hash preimage
  (through the format version byte or the id itself) or in the signed message. A verifier
  must never pick the algorithm from an unauthenticated field and then trust the result,
  because that would allow downgrade or cross-algorithm confusion.

The code reserves identifiers only. No format carries them yet and no behaviour changes.
That is justified because the ids must exist, uniquely, before any of the wire-format fields
below can be added.

### 2. Wire-format fields needing algorithm ids before the v1 freeze

| Format | Field to add | Why |
| --- | --- | --- |
| QR payload, offline receipt (key `7`) | Sub-key `6` `sig_alg` (uint, default `0x01` Ed25519). Size `attester` and `attester_sig` by algorithm instead of fixing them at 32 and 64 bytes | Receipts are the only place an attester signature reaches a verifier without the chain. A PQ receipt must be distinguishable |
| QR payload, patient signature (key `9`) | Key `11` `patient_sig_alg` (uint, default Ed25519) | Same reason |
| QR payload, commitment | Keep `lrc_version` (key `5`) as the commitment *suite* id: each LRC version fixes its hash. Document that a new hash means a new LRC version, never a new hash id beside an old version | Avoids two fields that could disagree |
| Merkle batch (`anchor_batch`) | A `hash_alg`/version byte in the leaf and node prefixes, or an argument to a future `anchor_batch_v2` | Today nothing identifies the tree hash |
| Offline trust bundle (ADR-0014) | `sig_alg` per listed key and for the bundle signature | Lets a bundle list PQ attester keys |
| `@lafiya/multisig-auth` | Mirror `SignerKey` variants instead of assuming Ed25519 | Follows the contract |

These are reserved in [qr-payload-v1.md](../specs/qr-payload-v1.md) (§4.4) so no other field
can take the keys. The contract interfaces need no change: `record_hash` is opaque, and both
`SignerKey` and attester addresses are already extensible.

### 3. Dual-hash transition windows

Moving commitments from SHA-256 to a successor H′ follows these rules:

1. **New suite.** Define LRC-*n* = the same canonicalization with H′ and a new version byte
   in the preimage. Add vectors for Rust, TypeScript and Python, and a stability-policy
   entry.
2. **Open the window.** Wallets compute both commitments for every new or re-issued card,
   using the same salt version (ADR-0018). Each commitment is domain-separated by its version
   byte, so sharing the salt reveals nothing new. Attesters attest **both** record hashes in
   one transaction. Cards carry the new `lrc_version`. Verifiers accept either suite and
   prefer the newer one.
3. **Re-attest live records.** During the window, each patient's next visit, or a batch job
   from the wallet, adds the H′ commitment for existing records. The SHA-256 attestation is
   left in place, not revoked.
4. **Close the window.** After a sunset announced at least **2 minor releases** ahead,
   attesters stop attesting SHA-256 commitments. Verifiers keep verifying existing SHA-256
   attestations for the full 10-year card lifetime unless the hash is actually broken (see
   the playbook).
5. **Never orphan.** No verifier may drop support for a suite that still has unexpired cards
   in the field.

Signatures follow the same shape. During the window, keys are **hybrid** (`0x20`, both
signatures must verify), so security never drops below the stronger scheme. After it, keys
are post-quantum only.

### 4. Why historical attestations survive a signature break

An attestation's authority comes from its inclusion in Stellar ledger history at a known
ledger and time, not from a signature a verifier re-checks. If Ed25519 were broken, an
attacker could authorize *new* transactions as an attester, but could not rewrite the ledger
history that consensus has already finalized. The playbook therefore adds a **trust cutoff
ledger**: attestations recorded before the cutoff stay valid, and authorizations after it
must come from post-quantum or hybrid signers. Offline receipts and patient signatures have
no ledger behind them, so receipts signed with a broken scheme lose their offline validity
and must be corroborated online.

## Soroban host-function availability and cost

| Primitive | Host function | Notes |
| --- | --- | --- |
| SHA-256 | `crypto().sha256` | Metered linearly in input size. Used by `lafiya-commitment::soroban` |
| Keccak-256 | `crypto().keccak256` | Comparable linear cost. The only SHA-256 alternative available natively |
| Ed25519 verify | `crypto().ed25519_verify` | Fixed cost plus a per-byte cost for the message |
| ECDSA P-256 verify | `crypto().secp256r1_verify` (Protocol 21) | Noticeably more expensive than Ed25519. Already used by `multisig-account` |
| ECDSA secp256k1 | `crypto().secp256k1_recover` | Recovery, not verification |
| BLS12-381 | `crypto().bls12_381_*` (Protocol 22) | Pairing-based. Aggregation possible, pairings are expensive |
| BN254, Poseidon | CAP-0074 / CAP-0075 | Confirm availability on the target network before relying on them |
| SHA3-256, SHA-512/256, BLAKE2/3 | None | Must run as guest Wasm, metered per instruction. Workable for one 32-byte digest, costly for large inputs |
| ML-DSA, SLH-DSA, FN-DSA | None | Guest-Wasm verification is the only option today. Expect it to cost orders of magnitude more than `ed25519_verify` and to approach per-transaction CPU limits. Public keys of 0.9–1.3 KB and signatures of 0.7–7.9 KB also raise ledger-entry and transaction-size fees |

Consequences for Lafiya:

- **Hash agility is cheap on chain.** No contract hashes records. Commitments are recomputed
  off-chain by verifiers, so changing the hash needs no host function at all. Only a future
  on-chain Merkle or batch verifier would. Keep it that way.
- **Signature agility depends on Stellar.** Attester authorization runs on Stellar's account
  model. The realistic post-quantum path is attesters moving to contract accounts
  (`multisig-account`) and adding a PQ `SignerKey` variant once a host function or an
  affordable Wasm verifier exists. Until then the hybrid variant cannot be enforced on chain.
- **QR size limits post-quantum signatures in cards.** An ML-DSA-44 signature (2,420 bytes)
  alone exceeds every QR profile budget (version ≤ 20 in §5 of the QR spec). Post-quantum
  offline receipts need an out-of-band or detached-signature design, such as fetching the
  receipt by hash or using FN-DSA-512 (about 666 bytes) in a larger profile.
- Before activating any id, measure CPU, memory and fees with the budgets harness
  (`budgets/baseline.json`) and record them in the activating ADR.

## Migration playbook and timeline triggers

| Stage | Trigger | Actions |
| --- | --- | --- |
| **T0 Watch** (now) | Default | Keep this inventory current; any new primitive must add an inventory row and an id. Track NIST, IETF CFRG and Stellar CAPs. |
| **T1 Prepare** | Any of: a NIST or IETF deprecation notice for an active algorithm; a published attack cutting its security margin below 112 bits; Stellar shipping a PQ host function; credible CRQC estimates inside the card lifetime of 10 years. Planning anchor: NIST IR 8547 deprecates 112-bit quantum-vulnerable signatures after 2030 and disallows them after 2035, so for Ed25519 T1 starts no later than **2028** | Write the activating ADR and add vectors in all languages. Ship verifier support first, behind the id, with no issuance yet. |
| **T2 Dual window** | Verifier support deployed to most active verifiers (from release-manifest adoption data), at the latest 2 minor releases after T1 | Wallets and attesters issue both suites (hash) or hybrid signatures. Re-attest live records. |
| **T3 Sunset** | Announced at least 2 minor releases in advance, after re-attestation coverage targets are met | Stop issuing the old suite. Keep verifying it. |
| **T4 Historical** | Card lifetime elapsed, or an actual break | **Break of a hash** (practical collisions or second preimages): mark the suite *historical*. Accept it only for attestations recorded before a published cutoff ledger, and surface a "legacy algorithm" reason in verdicts. **Break of a signature scheme**: set the trust cutoff ledger (§4), require PQ or hybrid signers for all later authorizations, invalidate offline receipts in that scheme, and emergency-stop affected registries ([ADR-0012: coordinated emergency stop](0012-coordinated-emergency-stop.md)) until attesters have rotated. |

Ownership: maintainers review the T0 inventory at every minor release. Moving between stages
requires an ADR update, and any change verifiers can see requires a `CHANGELOG.md` entry.

## Alternatives considered

### Self-describing multihash or multicodec everywhere

Rejected for on-chain fields: `record_hash` stays a fixed `BytesN<32>` lookup key, and a
varint prefix would change storage layout and indexing. Multiformats remain fine in
off-chain formats such as VCs, which already carry `cryptosuite`.

### Defer until a break is announced

Rejected: retrofitting algorithm fields after the v1 freeze breaks every deployed verifier,
and the card lifetime means today's cards are still valid in the 2030s.

### Move to post-quantum signatures now

Rejected for now: there are no host functions, the costs and QR sizes are prohibitive, and
Stellar's own transaction signatures, the root of on-chain trust, are still Ed25519.

## Consequences

### Positive

- Every primitive has an owner row, an identifier and a migration path.
- Historical attestations stay verifiable through every stage short of an actual break, and
  through a signature break by means of the trust cutoff ledger.
- The v1 freeze can include the few fields that make later migration non-breaking.

### Trade-offs and risks

- Dual windows double attestation cost and storage while open.
- Post-quantum readiness for attestation integrity ultimately depends on Stellar protocol
  work outside this project.
- The registry must be kept in sync with `alg.rs` by review. No automated check exists yet.

## Follow-up

- Specify and implement the reserved QR fields (receipt `sig_alg`, `patient_sig_alg`) before
  the QR v1 freeze, with vectors in `lafiya-qr`.
- Add a version byte to the Merkle batch leaf and node prefixes before `anchor_batch` is
  frozen.
- Spike: measure guest-Wasm ML-DSA-44 and FN-DSA-512 verification cost under the Soroban
  budget.
- Add an "algorithm identifiers" surface to the stability policy's enforcement checks.

## References

- [Stability policy](../stability-policy.md), [QR payload v1](../specs/qr-payload-v1.md),
  [ADR-0008](0008-record-commitment-canonicalization.md),
  [ADR-0012 LRC-2](0012-lrc2-selective-disclosure.md),
  [ADR-0014](0014-offline-attestation-receipts.md),
  [ADR-0018](0018-deterministic-salt-derivation.md)
- FIPS 204 (ML-DSA), FIPS 205 (SLH-DSA), NIST IR 8547 (transition to post-quantum
  cryptography standards)
- Issue #564
