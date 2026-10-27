# ADR-0018: Deterministic salt derivation (HKDF-SHA256) from a patient master secret

- **Status:** Proposed
- **Date:** 2026-10-17
- **Deciders:** Lafiya contract maintainers

> **Terminology:** shared terms (*attester*, *record commitment*, *schema version*, …) are
> defined canonically in the [glossary](../glossary.md). Link there rather than redefining them.

## Context

[ADR-0008](0008-record-commitment-protocol.md) requires a high-entropy salt in every record
commitment so low-entropy fields (blood group, genotype) cannot be recovered by dictionary
attack, and `lafiya-commitment` implements this as `commit_v1_salted(salt, fields)` with a
32-byte `Salt` prepended to the LRC-1 payload. Neither says where the salt comes from.

That choice decides whether a record can be re-verified after the patient's device is lost:

- A **random salt per record** must be backed up for every record. Lose the backup and the
  commitment can never be opened again, so the card stops verifying.
- A **salt derived from the record** (for example, a hash of its fields) is recomputable by
  anyone who can guess the fields, which brings the dictionary attack back.

We want salts that are as unguessable as random ones but can be recomputed from one backed-up
secret.

## Decision

### Derivation

Each patient wallet holds one **master secret** `MS`: 32 to 64 bytes from a CSPRNG. Every
record salt is derived from it with HKDF-SHA256 ([RFC 5869]):

```text
PRK  = HKDF-Extract(salt = "lafiya:salt-derivation:v1", IKM = MS)

info = "lafiya:record-salt"            18 bytes, ASCII
    || 0x01                            info layout version (SALT_DERIVATION_V1)
    || u32be(len(record_id))
    || record_id                       1..=256 bytes
    || u32be(salt_version)             1..=2^32-1

salt = HKDF-Expand(PRK, info, L = 32)
```

- `record_id` is an opaque, non-secret, stable identifier for the logical record in the
  wallet (for example `rec-0001`). Textual ids are UTF-8 in NFC. It never appears on chain.
- `salt_version` starts at 1 and identifies one commitment of that record (see
  [Rotation](#rotation)). Version 0 is invalid.
- The 32-byte `salt` is the `Salt` passed to `commit_v1_salted`. Where a profile carries a
  16-byte salt (the JSON profile in ADR-0008), it is the first 16 bytes of the same output,
  which equals `HKDF-Expand(PRK, info, 16)`.
- Inputs outside these ranges are rejected, never truncated or padded.

The wallet also computes a public **master-secret fingerprint** and stores it next to each
commitment, so after a master-secret rotation it knows which secret opens which commitment:

```text
fingerprint = HKDF-Expand(PRK, "lafiya:master-secret-fingerprint", L = 8)
```

### Rotation

**Salt rotation, per record.** A new commitment for a record always uses the next
`salt_version`. That covers updated content, re-issuing a card after a salt was shown to a
verifier, and moving to a new commitment version. The new commitment is attested normally and
supersedes the old attestation through the registry's existing history and supersession
rules. The two commitments are unlinkable on chain because the salts are independent PRF
outputs. Only the wallet's record index `(record_id, salt_version, commitment, fingerprint)`
links them. A `(record_id, salt_version)` pair must never commit to two different contents:
that would reuse a salt, and anyone who learns one opening could then attack the other.

**Master-secret rotation.** When `MS` may be compromised, or as routine hygiene, the wallet
generates `MS'`, re-derives the salt for every live record at its next `salt_version` under
`MS'`, has each record re-attested, and revokes the old attestations. Commitments made under
`MS` can still be opened only while `MS` is kept. A wallet that archives `MS` (marked by its
fingerprint) can still open historical commitments. Otherwise they become unopenable, which is
the intended outcome after a compromise.

### Implementations and vectors

| Language | Code | Test |
| --- | --- | --- |
| Rust | `crates/lafiya-commitment/src/salt.rs` (`derive_salt`, `master_secret_fingerprint`; `no_std`, HMAC built on `sha2`) | `cargo test -p lafiya-commitment` |
| TypeScript | `crates/lafiya-commitment/reference-ts/salt.ts` (Node `hkdfSync`) | `node reference-ts/salt.test.ts` |
| Python | `crates/lafiya-commitment/reference-py/salt.py` (stdlib `hmac`) | `python reference-py/salt_test.py` |

All three check `crates/lafiya-commitment/vectors/salt-derivation-vectors.json`: RFC 5869
test cases 1 and 3 for the HKDF itself, eight derivations (including the exact `info` bytes,
the fingerprint, and the resulting `commit_v1_salted` commitment for three of them), and five
invalid inputs. CI runs the Rust vectors with the workspace tests and the TypeScript and
Python vectors in the `commitment-vectors` job. The Rust and Python implementations write
HMAC out by hand. The TypeScript one uses Node's native HKDF, so the vectors also cross-check
two independent HKDF implementations.

## Security argument

1. **Unguessable salts.** HKDF-Expand is a PRF keyed by `PRK` under the standard assumption
   that HMAC-SHA256 is a PRF, and HKDF-Extract turns a uniformly random `MS` of at least 256
   bits into a uniformly distributed `PRK`. Without `MS`, every salt is indistinguishable from
   32 random bytes, so a dictionary attack on a commitment must guess a 256-bit salt, exactly
   as with random salts.
2. **Independent salts.** The `info` encoding is injective: a fixed tag, a fixed version byte,
   a length-prefixed id and a fixed-width version. So distinct `(record_id, salt_version)`
   pairs give distinct PRF inputs and independent outputs. Disclosing one salt to a verifier
   reveals nothing about any other salt, and a rotated commitment cannot be linked to its
   predecessor. The `record-id-prefix` vector pins the length prefix.
3. **Domain separation.** The extract salt binds `PRK` to this purpose, and the salt and
   fingerprint use distinct `info` tags. `MS` must not be used directly for anything else.
   Any other key derived from it needs a different HKDF salt.
4. **Fingerprint.** The fingerprint is a separate PRF output, so it reveals nothing about the
   salts. Its 64 bits are enough to tell a wallet's few secrets apart but give no help in
   brute-forcing a 256-bit `MS`.
5. **Entropy is a precondition.** The length check cannot measure entropy. `MS` must come from
   a CSPRNG, never directly from a PIN or password. A user-memorable recovery code must be
   stretched with a memory-hard KDF (out of scope here) before being used as `MS`, or the
   whole scheme falls back to the strength of that code.
6. **Determinism.** Determinism is the point, and it is also why `(record_id, salt_version)`
   reuse is forbidden (see [Rotation](#rotation)).

## Recovery and loss scenarios

| Scenario | Outcome | What the wallet does |
| --- | --- | --- |
| Device lost, `MS` backed up (recovery sheet, encrypted cloud backup, second device) | Every commitment can be opened again | Restore `MS`, check its fingerprint, and re-derive salts from the record index. Without the index, re-derive by scanning `salt_version = 1, 2, …` for each known `record_id` until the commitment matches the attested `card_ref`. Sequential record ids keep this scan cheap. |
| Device lost, no backup of `MS` | Existing commitments can never be opened again, so cards stop verifying | Generate a new `MS`, have the records re-attested as new commitments, and ask the attester to revoke the old attestations. Nothing private leaks: the old commitments stay hiding. |
| Record index lost, `MS` kept | Recoverable | As in the first row: scan versions per record id. |
| One salt disclosed (card opened for a verifier) | Only that commitment's contents are exposed to that verifier | When the patient wants unlinkability again, re-issue at `salt_version + 1`. |
| `MS` compromised | The attacker can derive every salt, so every commitment made under `MS` is open to dictionary attack. Integrity is unaffected: forging attestations still needs an attester key. | Rotate the master secret (see [Rotation](#rotation)) and revoke the old attestations. Commitments already on chain cannot be un-published, so their privacy loss is permanent. Treat this as a privacy incident. |
| Attester or chain data lost | Unrelated to salts | Salts depend only on wallet-side inputs. |

## Alternatives considered

### Random salt per record, backed up individually

This is the status quo implied by ADR-0008. It is as secure as derivation but turns every
record into its own backup obligation. It was rejected because losing any backup permanently
breaks re-verification of that record.

### Salt = H(MS || record_id) or HMAC without HKDF

Plain hashing gives no PRF guarantee if `MS` is not uniform and invites ad-hoc encodings.
HMAC alone would be sound, but HKDF is the standard, widely available construction (WebCrypto
`deriveBits`, Node `hkdfSync`, Python and Rust libraries) and separates extraction from
expansion cleanly.

### Deriving salts from the attester or on-chain data

This was rejected because anyone with chain access could then compute the salts.

## Consequences

### Positive

- One secret backs up every record. A card can be re-verified after device loss as long as
  `MS` survives.
- Salts keep full 256-bit unguessability, and rotated commitments are unlinkable.
- Interoperable across Rust, TypeScript and Python, pinned by shared vectors in CI.

### Trade-offs and risks

- `MS` is a single point of failure for privacy: its compromise opens every commitment made
  under it.
- Wallets must enforce strictly increasing `salt_version` per record.
- The `info` layout and constants are frozen for `SALT_DERIVATION_V1`. Any change needs a new
  layout version byte and new vectors, under the cryptographic-agility rules in
  [ADR-0019](0019-cryptographic-agility.md).

## Follow-up

- `lafiya-web` and the verifier apps: adopt `derive_salt`, store the record index with
  fingerprints, and implement the recovery scan.
- Specify the recovery-code format (mnemonic plus memory-hard KDF) in a later ADR.

## References

- [RFC 5869] HMAC-based Extract-and-Expand Key Derivation Function (HKDF)
- H. Krawczyk, "Cryptographic Extraction and Key Derivation: The HKDF Scheme", CRYPTO 2010
- [ADR-0008](0008-record-commitment-protocol.md), [ADR-0012: per-attestation blinded commitments](0012-per-attestation-blinded-commitments.md)
- Issue #563

[RFC 5869]: https://www.rfc-editor.org/rfc/rfc5869
