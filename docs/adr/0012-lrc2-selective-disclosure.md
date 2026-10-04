# ADR-0012: LRC-2 — Per-field Merkle commitments for selective disclosure

- **Status:** Proposed
- **Date:** 2026-09-27
- **Deciders:** Lafiya contract maintainers

## Context

ADR-0008 specifies LRC-1: a single SHA-256 commitment over all record fields. To verify any
field, a verifier must receive the entire record payload. That is acceptable for a full-record
check (e.g. an emergency room clinician who sees everything) but it contradicts the minimal-
disclosure principle in Lafiya's stated privacy design and in the Nigeria Data Protection Act
(2023). A paramedic needs blood group and allergies; a pharmacist needs current medications;
neither needs HIV status or mental-health history.

[Issue #388](https://github.com/Lafiya-xyz/Lafiya-contract/issues/388) asks for a selective-
disclosure commitment scheme that:

- lets a holder disclose an arbitrary subset of fields with a verifiable proof;
- fits the resulting disclosure package in a QR code (version ≤ 25, EC level M, ≈ 900 bytes
  encoded in base45 / alphanumeric mode);
- keeps the same 32-byte on-chain slot so `attestation-registry` needs no changes;
- is implementable in Rust, TypeScript, and Python without exotic dependencies.

This ADR compares Merkle-based disclosure, SD-JWT-style salted-hash lists, and BBS+ signatures,
then selects the approach for a prototype. The prototype lives in
`crates/lafiya-commitment` behind the `lrc2` feature flag.

---

## Selective-disclosure approaches compared

### Option A — SD-JWT style: salted-hash list (chosen)

Each field `i` gets an independent hash:

```
leaf_i = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt_i || encode(field_i))
```

The record commitment is the hash of the **sorted, concatenated leaf hashes**:

```
record_commitment = SHA-256("lafiya:lrc2:root" || leaf_0 || leaf_1 || ... || leaf_{n-1})
```

A disclosure package for a subset S is:

```
{ version: 0x02,
  root: <32 bytes, the record_commitment>,
  disclosures: [ { index: u8, salt: [u8; 32], value: FieldValue } for i in S ],
  undisclosed_leaves: [ leaf_i for i not in S ]   // optional: full Merkle audit path
}
```

Verifier algorithm:
1. Recompute each disclosed `leaf_i` from `(index, salt, value)`.
2. Reconstruct the full leaf list (disclosed leaves + undisclosed leaves supplied in the package).
3. Recompute `record_commitment` and assert it matches the attested root on-chain.
4. Confirm the root has a valid attestation via `attestation-registry.get_attestation`.

**Size analysis — "paramedic view" (3 disclosed fields out of 8 total):**

| Component | Size (bytes) |
|---|---|
| Version + root | 1 + 32 = 33 |
| Per disclosure: index(1) + salt(32) + encoded value(≤ 50) | 3 × 83 ≈ 249 |
| Undisclosed leaves: 5 × 32 | 160 |
| Framing / CBOR overhead | ≈ 30 |
| **Total** | **≈ 472 bytes** |

Base45-encoded: ≈ 577 chars → fits QR version 13 at EC level M (capacity 894 chars). Well
within the version ≤ 25 budget, and small enough for a version ≤ 15 minimal profile.

**Why this over a pure Merkle tree:**

For Lafiya's record sizes (4–16 fields), the per-field cost of a binary Merkle path is
`log2(16) = 4` hashes = 128 bytes. The total for 3 disclosed out of 8 is comparable to the
flat list: 3 × (1 + 32 + 50) + 5 × 32 = 409 + 160 + overhead ≈ 600 bytes. The flat list is
simpler to implement and audit — no tree-building logic, no sibling-ordering conventions — and
slightly smaller when the majority of leaves are undisclosed. The only advantage of a full
Merkle tree is revealing fewer undisclosed hashes (only the audit path, not all siblings), but
for ≤ 16 leaves the difference is at most 64 bytes, which does not justify the extra complexity
at this stage.

---

### Option B — Merkle tree with per-leaf salts

A binary Merkle tree where each leaf is:

```
leaf_i = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt_i || encode(field_i))
```

and internal nodes are:

```
node = SHA-256("lafiya:lrc2:node" || left || right)
```

The record commitment is the Merkle root.

A disclosure proof for a subset S is the standard set of Merkle audit paths: for each disclosed
`i`, the sibling hashes at each level up to the root.

**Pros:**
- Sublinear proof size for large records: `O(log n)` hashes per disclosed field regardless of `n`.
- Well-understood cryptographic primitive.
- For very large records (n > 64) clearly outperforms a flat list.

**Cons:**
- For Lafiya's range of 4–16 fields, log₂(16) = 4 hashes per path ≈ same total bytes as the
  flat list, but with higher implementation complexity (tree construction, padding to next power
  of two, sibling direction bits).
- Requires a canonical tree-padding rule when `n` is not a power of two (e.g. RFC 6962 style:
  fill with a fixed zero leaf, or truncate). Different implementations that choose different
  padding will silently produce different roots.
- Sibling ordering (is the left or right child the disclosed node?) adds a direction bit per
  level and another source of cross-implementation divergence.

**Decision:** deferred. If Lafiya records grow beyond ~32 fields, revisit with a proper binary
Merkle tree. At current sizes Option A is simpler and nearly the same size.

---

### Option C — BBS+ signatures

BBS+ (Boneh-Boyen-Shacham variant with "proof of knowledge") lets a holder derive a
zero-knowledge proof of a subset of signed messages without revealing the others. The
verifier never sees undisclosed fields; even undisclosed leaves are not transmitted.

**Pros:**
- True zero-knowledge: undisclosed fields leave no trace, not even a hash.
- Field count doesn't affect proof size (typically 48–96 bytes on BLS12-381).
- Strong academic foundations; standardization ongoing (IETF BBS draft).

**Cons:**
- Requires BLS12-381 or BLS12-381 + pairing arithmetic, which is absent from
  `soroban-sdk` and from Node's built-in `crypto`. An off-chain verifier would need
  an additional Wasm or native crypto library.
- Proof generation requires the holder to interact with a BBS+ library; the current
  `lafiya-web` stack (TypeScript) would need `@noble/curves` or similar.
- The issuer (attester) would need to sign each field at attestation time, which
  changes the on-chain call surface: today `attest` takes a single 32-byte hash;
  with BBS+ it would need to take or commit to the ordered field hashes.
- Standardization is still in draft (as of 2026); the spec is still evolving.
- Deployment in a QR code: a BBS+ proof is ≈ 112 bytes, but the context needed for
  verification (public key, field count, disclosed field indices) adds overhead; total
  is comparable to the Merkle/flat approach.

**Decision:** rejected for the LRC-2 prototype. BBS+ is the right long-term answer once
verifier tooling matures and if regulatory auditors require true zero-knowledge proofs for
undisclosed fields. Flag for revisit at M4 (mainnet).

---

### Comparison matrix

| Criterion | SD-JWT flat list (chosen) | Binary Merkle tree | BBS+ signatures |
|---|---|---|---|
| On-chain change required | None (same 32-byte root) | None | Likely (attester signs field hashes) |
| Proof size: 3/8 fields | ≈ 472 bytes | ≈ 490 bytes | ≈ 200 bytes + VK |
| Fits QR v ≤ 15 (min profile) | ✓ | ✓ | ✓ (but verifier key adds to QR) |
| Implementation complexity | Low (SHA-256 + sort) | Medium (tree build + path) | High (BLS12-381 pairing) |
| Cross-language compatibility | High (SHA-256 is universal) | Medium (padding convention) | Low (library gap in JS/WASM) |
| Zero-knowledge of undisclosed | No (hash is revealed) | No (sibling hash is revealed) | Yes |
| Standardization maturity | Informal (Lafiya-specific) | RFC 6962 (for CT) | IETF draft |
| Dependency footprint | None beyond SHA-256 | None beyond SHA-256 | BLS12-381 library required |

---

## Decision

Adopt **LRC-2 (Lafiya Record Commitment v2)** using the SD-JWT-style salted-hash flat list:

```
leaf_i  = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt_i[32] || encode_lrc1(field_i))
root    = SHA-256("lafiya:lrc2:root" || leaf_0 || leaf_1 || ... || leaf_{n-1})
```

- `encode_lrc1(field_i)` is the LRC-1 field encoding defined in ADR-0008: the same tag +
  length-prefix scheme, applied to a single field.
- `salt_i` is a 32-byte cryptographically random value generated by the holder at record
  creation time and stored by `lafiya-web` alongside the field values.
- `u8(i)` is the zero-based field index, encoded as a single byte. This prevents two
  fields with identical values from producing the same leaf regardless of salt.
- The root is the value stored on-chain via `attest(attester, root)`.
- **Version byte:** LRC-2 uses version byte `0x02` in contexts where a version must be
  explicit (e.g. QR payload, see ADR-0013). The version is NOT mixed into the root hash
  itself — mixing it in would mean a verifier with only the root cannot distinguish LRC-1
  from LRC-2 roots by inspection, which is already the case (the on-chain slot is opaque).

### Version-byte allocation (per ADR-0008's versioning rules)

| Byte | Scheme |
|---|---|
| `0x00` | Legacy/unversioned (reserved; never produced) |
| `0x01` | LRC-1 (whole-record SHA-256) |
| `0x02` | LRC-2 (salted per-field flat list) |
| `0x03–0xFF` | Reserved for future schemes |

### QR size budget

For the **minimal "paramedic" disclosure profile** (blood_group, allergies, emergency_contact
— 3 fields out of a 8-field schema):

| Component | Bytes |
|---|---|
| CBOR framing + version + root | 36 |
| 3 disclosures × (index:1 + salt:32 + value:≤50) | ≤ 249 |
| 5 undisclosed leaves × 32 | 160 |
| **Total** | **≤ 445** |

Base45-encoded (alphanumeric QR mode): `ceil(445 / 3 * 4)` ≈ **594 chars** — fits QR version
13 (EC level M, capacity 850). Maximum for full-record disclosure (all 8 fields): ≤ 8 × 83 +
36 = 700 bytes ≈ 934 chars — fits QR version 15 (EC level M, capacity 1062).

The "pharmacist" profile (3 medication fields from 8) has the same size budget as the
paramedic profile, so all common single-purpose profiles fit in QR version ≤ 15.

---

## Prototype

The prototype is at `crates/lafiya-commitment` behind the `lrc2` Cargo feature flag:

- `src/lrc2.rs` — Rust implementation of `commit_lrc2`, `make_leaf`, `make_root`,
  `verify_disclosure`.
- `vectors/lrc2-test-vectors.json` — shared cross-language test vectors.

See the [lafiya-commitment README](../../crates/lafiya-commitment/README.md) for how to
build and test.

---

## Threat analysis

- **Dictionary attack on undisclosed leaves.** An undisclosed leaf `SHA-256(domain || i || salt
  || encode(field))` is indistinguishable from a random value to any party that doesn't hold
  `salt_i`. This is the primary reason per-field salts are required; without them, an adversary
  with the root and some disclosed leaves could brute-force the remaining fields if they are
  low-entropy (e.g. blood group has only 8 possible values).
- **Leaf swapping.** The index byte `u8(i)` is hashed into each leaf, so an attacker cannot swap
  two leaves of the same value without changing the leaf hashes and therefore the root.
- **Cross-record replay.** The root is attested on-chain by a specific attester at a specific
  time. A verifier must check the attestation registry, not just the disclosure proof, to confirm
  the root was actually attested.
- **Salt reuse.** If the same salt is reused across two records, two leaves with the same
  `(index, field_value)` will have the same leaf hash, leaking that the field value is identical
  across records. `lafiya-web` MUST generate a fresh random 32-byte salt per field per
  record creation.
- **QR tampering.** See ADR-0013 (QR payload spec) for the optional patient signature that
  binds the disclosure package to the printed card, preventing substitution of a different
  (valid) disclosure package in the same QR position.
- **Bundle expiry (offline).** See ADR-0014 (offline attestation receipts) for how a responder
  without connectivity can verify that the disclosing party's attestation was valid as of a
  recent trust bundle.

---

## Consequences

### Positive
- A responder receives only the fields relevant to their role. No field not in the disclosure
  subset is transmitted.
- The same 32-byte on-chain slot is reused; `attestation-registry` is unchanged.
- No new cryptographic primitives beyond SHA-256; all three language implementations can use
  their standard library.

### Trade-offs and risks
- Undisclosed field hashes (leaves) are transmitted. An adversary who can observe many disclosures
  and knows the salt CAN verify a guess. This is acceptable only because the salt is secret and
  stored exclusively by the patient in `lafiya-web`'s encrypted store.
- LRC-2 requires `lafiya-web` to store salts per field per record. Salt loss means the patient
  can no longer produce a disclosure proof (though the root remains attested on-chain).
- The flat-list root is slightly larger than a binary Merkle root for partial disclosure of large
  records (n > 32), but Lafiya records are expected to stay well below this.

## Follow-up

- Specify the concrete field list and salt-storage model for each Lafiya record type in `lafiya-web`.
- Write the QR codec (ADR-0013) that embeds LRC-2 disclosure packages in a versioned binary payload.
- Write the offline receipt spec (ADR-0014) that references this disclosure format for offline-
  verifiable proofs.
- Revisit BBS+ once the IETF draft stabilizes and a production-quality JS/Wasm library is available.

## References

- [ADR-0008](0008-record-commitment-canonicalization.md) — LRC-1 canonicalization
- [Issue #388](https://github.com/Lafiya-xyz/Lafiya-contract/issues/388)
- [`crates/lafiya-commitment/src/lrc2.rs`](../../crates/lafiya-commitment/src/lrc2.rs) — Rust prototype
- [IETF BBS Signature Scheme draft](https://identity.foundation/bbs-signature/draft-irtf-cfrg-bbs-signatures.html)
- [RFC 6962 — Certificate Transparency Merkle tree](https://www.rfc-editor.org/rfc/rfc6962)
- [SD-JWT (IETF draft-ietf-oauth-selective-disclosure-jwt)](https://www.ietf.org/archive/id/draft-ietf-oauth-selective-disclosure-jwt-12.txt)
