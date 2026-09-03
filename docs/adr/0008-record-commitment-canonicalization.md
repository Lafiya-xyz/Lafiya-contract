# ADR-0008: Record commitment canonicalization and domain separation (LRC-1)

- **Status:** Proposed
- **Date:** 2026-08-18
- **Deciders:** Lafiya contract maintainers

## Context

ADR-0001 established that `attestation-registry` stores only an opaque `BytesN<32>`
`record_hash` and that "commitment construction must ... be defined by the off-chain
data model and threat model, including canonical serialization, domain separation, and
secret entropy or another hiding construction where required." That construction was
never specified. Today the web application, any verifier, and any future re-implementation
each have to guess the same byte-exact algorithm, or attested cards silently fail to verify.

[Issue #133](https://github.com/Lafiya-xyz/Lafiya-contract/issues/133) asks which
serialization and domain-separation scheme lets Rust, TypeScript, and verifier clients
reproduce the same commitment, and requests a comparison of at least two canonicalization
approaches, cross-language test vectors, a prototype, a threat analysis, and a migration
plan. This ADR is that decision; the prototype lives in
[`crates/lafiya-commitment`](../../crates/lafiya-commitment).

This ADR governs the *shape* of the off-chain commitment construction. It does not change
`attestation-registry` or `attester-registry`, which continue to treat `record_hash` as
opaque per ADR-0001.

## Decision

Adopt **LRC-1 (Lafiya Record Commitment v1)**:

```text
commitment = SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)
```

- `DOMAIN_TAG` is the fixed ASCII string `"lafiya:record-commitment"`. It ensures a
  Lafiya record commitment can never collide with, or be mistaken for, a hash produced
  by an unrelated protocol or a different Lafiya subsystem over similarly shaped bytes.
- `VERSION_V1` is the single byte `0x01`, identifying this canonicalization scheme.
  Binding the version into the hash input means an attacker cannot present a v1 payload
  as if it were hashed under a hypothetical weaker future/past scheme (or vice versa)
  without changing the resulting commitment.
- `canonical_payload` is produced by `encode_payload`: a **fixed-schema, length-prefixed
  binary encoding**, not general-purpose canonical JSON or CBOR. A record is an ordered
  list of fields whose order is part of the schema (defined once, per record type — never
  sorted at runtime). Each field encodes as a one-byte tag followed by its value:

  | Tag | Meaning | Encoding |
  | --- | --- | --- |
  | `0x00` | Absent | tag only — the source record does not contain this field |
  | `0x01` | Null | tag only — the field is present with an explicit null value |
  | `0x10` | Text | `u32` big-endian byte length + UTF-8 bytes (must be NFC-normalized) |
  | `0x11` | Int64 | 8-byte big-endian two's-complement `i64` |
  | `0x12` | Bool | one byte, `0x00`/`0x01` |
  | `0x13` | Bytes | `u32` big-endian byte length + raw bytes |

  There is no floating-point type. Timestamps are `Int64` Unix-epoch seconds, UTC,
  truncated to whole seconds.

This directly fixes the null/omitted/Unicode/ordering/timestamp rules the issue asked
about:

- **Absent vs. null** get different tags, so "field not collected" and "field explicitly
  cleared" can never hash identically.
- **Ordering** is fixed by the schema definition, not by a runtime sort — so there is no
  dependency on Unicode collation or key-comparison rules (the ambiguity that makes naive
  "canonical JSON" schemes hard to get byte-identical across languages).
- **Unicode**: text fields must be supplied in Normalization Form C. The reference
  implementations document this precondition; see "Edge cases and known gaps" below for
  why it isn't enforced in the Rust prototype.
- **Timestamps**: always whole-second Unix epoch integers, encoded as a fixed-width
  8-byte integer — no locale, timezone, or sub-second precision ambiguity, and no
  floating-point rounding.

## Alternatives considered

### Canonical JSON (RFC 8785, JCS)

JCS sorts object keys by UTF-16 code unit and defines a specific number-to-string
algorithm borrowed from ECMA-262. It is a real, published standard and a reasonable
default for genuinely schema-less payloads.

Rejected as the primary scheme because Lafiya records are not schema-less: reproducing
JCS's number formatting exactly (in particular for values outside the safely-representable
integer range, or accidentally-float-typed fields) is a well-known source of
cross-implementation divergence, and getting it byte-identical in Rust requires either a
bespoke encoder or an extra dependency this spike avoided introducing. It also does not
by itself solve the null-vs-absent distinction — that still needs an explicit protocol
convention layered on top.

### Deterministic CBOR (RFC 8949 §4.2, "Core Deterministic Encoding")

CBOR's deterministic mode fixes map key ordering (by encoded byte length, then
lexicographic) and requires the shortest-form integer encoding. It handles binary data
natively (no base64 inflation) and has mature libraries in both ecosystems.

Rejected as the primary scheme for the same core reason as JCS: it is designed for
schema-less, self-describing maps, and Lafiya's records are not that. Its shortest-form
integer rule is an extra place implementations can disagree if two libraries pick
different "minimal" encodings for edge values. It remains a reasonable choice if the
Lafiya record shape becomes genuinely open-ended (e.g., extensible key/value bags); see
"Follow-up."

### Fixed-schema binary encoding (chosen)

Because the record shape is defined by Lafiya's own schema (not third-party or
user-defined), a general canonicalization algorithm is solving a harder problem than the
one that exists. Fixing field order in the schema and giving every field an explicit
presence tag removes the two biggest sources of cross-language disagreement (key
ordering, null handling) by construction rather than by careful library configuration.
The trade-off is that adding, removing, or reordering fields requires a new schema
version — which this ADR requires anyway, via `VERSION_V1`.

### Comparison matrix

| Criterion | Canonical JSON (JCS) | Deterministic CBOR | Fixed-schema binary (chosen) |
| --- | --- | --- | --- |
| Determinism | High, if number formatting is implemented exactly per spec | High, if shortest-form integer rule is implemented exactly | High by construction — no formatting rules to replicate |
| Privacy | No inherent hiding; same low-entropy-field risk as any hash | Same | Same (see threat analysis) |
| Backward compatibility | New fields need explicit optionality convention | Same | New fields require a schema version bump (explicit, not implicit) |
| Implementation complexity | Medium–high (exact number/string formatting) | Medium (integer minimality, key ordering) | Low (fixed order, explicit lengths, no floats) |
| Verifier usability | Requires a spec-compliant JCS library or careful reimplementation | Requires a CBOR library configured for deterministic mode | A verifier only needs to know the schema and this ADR — no general-purpose codec required |

## Threat analysis

- **Low-entropy field guessing (dictionary attack).** A commitment over predictable
  fields (e.g. a common `record_type` and a narrow timestamp window) can be brute-forced
  by an attacker who can guess the field values, regardless of how the fields are
  canonicalized — hashing alone does not hide low-entropy inputs. ADR-0001 already flags
  this. LRC-1 does not solve it; a record type that needs hiding, not just integrity,
  must include a high-entropy field (e.g. a random nonce or salt) among its schema fields
  so the commitment cannot be reconstructed by guessing the visible fields alone. This is
  called out explicitly as follow-up work for the record schema design, not something a
  canonicalization scheme can fix on its own.
- **Domain separation / cross-protocol reuse.** Without `DOMAIN_TAG`, a value that is a
  valid LRC-1 commitment could coincidentally (or adversarially) also be a valid hash
  output for an unrelated purpose elsewhere in the system, letting a party pass off a
  hash computed for one purpose as if it were computed for another. The fixed domain tag,
  hashed first, removes this ambiguity: the SHA-256 input is unique to
  "Lafiya record commitment" regardless of what else might hash to the same
  `canonical_payload` bytes in a different context.
- **Version confusion / downgrade.** Because `VERSION_V1` is part of the hashed input
  (not metadata carried alongside the hash), an attacker cannot take a v1 payload and
  claim it was produced by a different, weaker scheme without the commitment itself
  changing. `VERSION_LEGACY_UNVERSIONED = 0x00` is reserved and will never be assigned to
  an actual scheme, so it can never be produced by a correct v1-or-later implementation —
  see "Existing commitment compatibility."
- **Canonicalization disagreement (availability/correctness, not confidentiality).** If
  producer and verifier disagree on field order, presence tags, or Unicode form, they
  compute different commitments for what a human would consider "the same" record. This
  is not a secrecy break, but it silently breaks verification (a legitimate record fails
  to match). The fixed field order and explicit presence tags remove the two largest
  sources of this; the Unicode precondition (below) is the one gap this spike leaves
  partially open.
- **Length-prefix ambiguity.** Explicit `u32` length prefixes (rather than delimiters)
  mean no field value can be crafted to be misread as a tag or as the start of the next
  field, which rules out a class of encoding-ambiguity attacks that affect delimiter- or
  escaping-based formats.

## Existing commitment compatibility

`attestation-registry` has never computed or interpreted `record_hash` — it is accepted
as an opaque `BytesN<32>` (ADR-0001). Any commitments already recorded on-chain were
therefore produced by whatever ad hoc construction the caller used at the time, which is
undocumented and not guaranteed to follow any particular algorithm.

This ADR does not assert, and a verifier must not assume, that pre-existing commitments
were produced by LRC-1. They are treated as **version `0x00`, "legacy/unversioned"** — a
reserved value that this scheme never produces — meaning: opaque, with no defined
preimage relationship, to be verified only via whatever process (if any) was used to
create them originally. No migration of past commitments is proposed or required; the
contracts' storage and behavior are unaffected either way, since they never inspected the
hash.

## Consequences

### Positive

- The web application, a verifier, and any future re-implementation have a single,
  precisely specified, dependency-light algorithm to implement, with matching
  cross-language test vectors to check against.
- Domain separation and an explicit version byte remove two concrete attack classes
  (cross-protocol reuse, version downgrade) without adding runtime cost.
- The fixed-schema encoding is simple enough to audit by hand — there is no general
  parser or codec whose edge cases need to be trusted.

### Trade-offs and risks

- Adding, removing, or reordering fields requires a new version byte and, in the general
  case, a way for a verifier to know which schema a given version implies. This ADR
  defines the byte-level container, not the specific field list for any one Lafiya record
  type — that belongs to the record schema itself and is `lafiya-web`'s responsibility to
  define and version.
- LRC-1 provides integrity and domain separation, not confidentiality: it does not by
  itself defend against low-entropy field guessing (see threat analysis).
- Because it is fixed-schema, LRC-1 is a poor fit if Lafiya records become genuinely
  open-ended (arbitrary user-defined keys); deterministic CBOR would be the better choice
  at that point (see Follow-up).

## Recommended protocol and migration plan

1. `lafiya-web` defines the concrete field list and order for each record type it needs
   to commit to, and encodes it with `encode_payload` / `commit_v1` (or the TypeScript
   equivalent) exactly as specified here.
2. New attestations use LRC-1 (`VERSION_V1`) going forward. No on-chain change is
   required — `attest` already accepts any `BytesN<32>`.
3. Any API or documentation that surfaces a commitment to a verifier should also surface
   which version produced it (e.g. `0x00` = legacy/unversioned, `0x01` = LRC-1), so a
   verifier knows whether — and how — it can attempt to reproduce the hash from a
   claimed preimage. This repository's contracts do not need this metadata; it is a
   concern for `lafiya-web` / `lafiya-verifier` response payloads.
4. If a future record type needs open-ended or nested fields that a fixed schema cannot
   express cleanly, define `VERSION_V2` using deterministic CBOR rather than extending
   LRC-1's tag set indefinitely.

## Follow-up

- Specify the concrete field list, order, and any hiding/salting requirements for each
  real Lafiya record type in `lafiya-web`'s data-model documentation, built on top of
  this ADR's container format.
- Add Unicode NFC validation or normalization to a production Rust implementation (e.g.
  via a `unicode-normalization`-class crate); the prototype in `crates/lafiya-commitment`
  documents the precondition but does not enforce it, to avoid adding a dependency purely
  for this spike.
- Publish a shared hashing package (or synchronized reference implementations) consumed
  by `lafiya-web` and `lafiya-verifier`, seeded from `crates/lafiya-commitment` and
  `crates/lafiya-commitment/reference-ts`.
- Decide, in `lafiya-web`, whether any record type needs a high-entropy salt field to
  defend against low-entropy dictionary attacks, per the threat analysis above.

## References

- [`docs/adr/0001-hash-only-on-chain-footprint.md`](0001-hash-only-on-chain-footprint.md)
- [`crates/lafiya-commitment/src/lib.rs`](../../crates/lafiya-commitment/src/lib.rs) — Rust reference implementation
- [`crates/lafiya-commitment/reference-ts/lrc1.ts`](../../crates/lafiya-commitment/reference-ts/lrc1.ts) — TypeScript reference implementation
- [`crates/lafiya-commitment/vectors/lrc1-test-vectors.json`](../../crates/lafiya-commitment/vectors/lrc1-test-vectors.json) — shared cross-language test vectors
- [Issue #133](https://github.com/Lafiya-xyz/Lafiya-contract/issues/133)
- [RFC 8785 — JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785)
- [RFC 8949 §4.2 — CBOR Core Deterministic Encoding Requirements](https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2)
