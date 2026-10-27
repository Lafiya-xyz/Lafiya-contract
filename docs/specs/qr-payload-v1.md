# Lafiya QR Payload v1 Specification

- **Status:** Draft
- **Version:** 1 (`0x01`)
- **Date:** 2026-09-27
- **Authors:** Lafiya contract maintainers
- **Related:** [Issue #389](https://github.com/Lafiya-xyz/Lafiya-contract/issues/389),
  [ADR-0008 (LRC-1)](../adr/0008-record-commitment-canonicalization.md),
  [ADR-0023 (LRC-2)](../adr/0023-lrc2-selective-disclosure.md),
  [ADR-0025 (offline receipts)](../adr/0025-offline-attestation-receipts.md)

---

## 1. Purpose

A Lafiya emergency card is presented as a QR code. This document specifies exactly what the
QR code encodes so that `lafiya-web`, `lafiya-verifier`, and any third-party responder app all
produce and consume the same bytes.

Without a spec, common failures include:
- A URL that trusts the web server rather than the chain (server compromise → fake "verified").
- Missing or wrong `network_id`, so a testnet card "verifies" on a mainnet verifier or the reverse.
- Oversized payloads that won't scan on cheap cameras used in low-resource settings.
- No version field, making format evolution impossible without breaking all existing cards.

---

## 2. Design principles

1. **Cryptographic verification, not server trust.** The QR contains enough data for a verifier
   to confirm the commitment on-chain without trusting the web server that printed the card.
2. **Minimal payload.** The minimal profile (online verification, no offline receipt) must fit in
   QR version ≤ 15 at EC level M (≈ 1,062 chars in alphanumeric mode ≈ 799 bytes binary).
3. **Version-first.** The first byte is always the version. An unknown version → hard reject.
4. **Network-scoped.** Every payload carries a network fingerprint derived from the Stellar
   network passphrase. A testnet payload MUST NOT verify successfully against a mainnet contract.
5. **Extensible.** Optional fields are CBOR-optional; new fields can be appended to the map
   without breaking old decoders (which ignore unknown keys).

---

## 3. Encoding stack

```
binary payload (CBOR map)
     │
     ▼
zlib deflate (RFC 1951) — optional, applied only when it reduces size ≥ 4 bytes
     │
     ▼
base45 (RFC 9285) — for QR alphanumeric mode (45-character alphabet)
     │
     ▼
QR code (ISO/IEC 18004), alphanumeric mode, EC level M (recommended default; see §5.4)
```

The `LFQR:` prefix (5 chars, analogous to the EU DCC `HC1:` prefix) precedes the base45 payload
in the QR string. A decoder MUST check for this prefix before attempting base45 decode.
If the prefix is absent the payload is invalid.

> **Why CBOR + base45?** This is the approach used by EU Digital COVID Certificates (DCC).
> CBOR is compact and self-describing; base45 is more space-efficient than base64 in QR
> alphanumeric mode (45 allowed chars vs 64 for base64, but QR alphanumeric encodes 2 chars
> per 11 bits vs 8 bits per char for binary). The net result is roughly 25 % more data capacity
> compared to encoding raw bytes as hex or base64 in alphanumeric mode.

---

## 4. Binary payload structure (CBOR)

The payload is a **CBOR map** (RFC 8949). Keys are **unsigned integers** (not text strings) to
minimize encoded size. All fields use their natural CBOR types.

### 4.1 Field table

| Key | Name | Type | Required | Description |
|-----|------|------|----------|-------------|
| `1` | `version` | uint | **Yes** | Always `1` for this spec version. |
| `2` | `network_id` | bytes(4) | **Yes** | First 4 bytes of SHA-256(network passphrase UTF-8). |
| `3` | `attestation_registry` | bytes(32) | **Yes** | Contract ID of the `attestation-registry`. |
| `4` | `card_ref` | bytes(32) | **Yes** | The attested record commitment (LRC-1 or LRC-2 root). |
| `5` | `lrc_version` | uint | No | `1` = LRC-1 whole-record, `2` = LRC-2 selective disclosure. Defaults to `1` if absent. |
| `6` | `disclosures` | array | No | LRC-2 disclosure package (see §4.2). Present only when `lrc_version = 2`. |
| `7` | `offline_receipt` | map | No | Signed attestation receipt for offline verification (see ADR-0025). |
| `8` | `fallback_url` | tstr | No | Untrusted display-only URL. Verifiers MUST NOT use this for verification. |
| `9` | `patient_sig` | bytes(64) | No | Ed25519 signature by the patient's key over the canonical encoding of fields 1–5. |
| `11` | `issued_at` | uint | No | Unix epoch seconds at which the card payload was issued (see §12). |
| `12` | `valid_until` | uint | No | Unix epoch seconds after which the payload MUST be treated as stale (see §12). |

### 4.2 LRC-2 disclosure package (key `6`)

Present only when `lrc_version = 2`. An array of CBOR maps, one per disclosed field:

| Sub-key | Name | Type | Description |
|---------|------|------|-------------|
| `1` | `index` | uint | Zero-based field index. |
| `2` | `salt` | bytes(32) | Per-field random salt. |
| `3` | `kind` | uint | Field type tag: 0=absent, 1=null, 16=text, 17=int64, 18=bool, 19=bytes. |
| `4` | `value` | tstr / int / bool / bytes | Field value (absent/null have no value). |

Additionally, undisclosed leaves are carried in a separate field:

| Key | Name | Type | Description |
|-----|------|------|-------------|
| `10` | `undisclosed_leaves` | array of bytes(32) | Undisclosed leaf hashes in ascending index order. |

### 4.3 Offline receipt (key `7`)

See ADR-0025. When present, this is a CBOR map with at minimum:

| Sub-key | Name | Type |
|---------|------|------|
| `1` | `v` | uint (= 1) |
| `2` | `attester` | bytes(32) |
| `3` | `attested_at` | uint (Unix epoch seconds) |
| `4` | `ledger_seq` | uint |
| `5` | `attester_sig` | bytes(64) |

### 4.4 Reserved for algorithm identifiers (ADR-0019)

The following keys are reserved so that algorithm identifiers can be added before the v1
freeze without colliding with other fields. Values come from the registry in
[ADR-0019](../adr/0019-cryptographic-agility.md) (`crates/lafiya-commitment/src/alg.rs`).
Until they are specified, encoders MUST NOT emit them. When absent, the algorithm is the one
implied by this spec: Ed25519 for signatures, and the hash fixed by `lrc_version` for
commitments. Decoders that predate these keys ignore them like any unknown key. That fails
safe, because a signature made with another algorithm then fails Ed25519 verification
instead of passing.

| Location | Key | Name | Type | Meaning |
|----------|-----|------|------|---------|
| Top-level map | `11` | `patient_sig_alg` | uint | Signature algorithm of `patient_sig` (key `9`). |
| Offline receipt (key `7`) | `6` | `sig_alg` | uint | Signature algorithm of `attester` / `attester_sig`. |

A new commitment hash is never signalled by a separate field: it is a new `lrc_version`,
because each LRC version fixes its hash function.

---

## 5. Size budgets

### 5.1 Minimal profile (online verification, LRC-1)

No disclosure package, no offline receipt. Just the chain reference and registry address.

| Component | CBOR bytes |
|-----------|-----------|
| Map header + 5 keys | 8 |
| version (key 1) | 2 |
| network_id (key 2) | 5 |
| attestation_registry (key 3) | 33 |
| card_ref (key 4) | 33 |
| lrc_version (key 5) | 2 |
| fallback_url (key 8, ≤ 80 chars) | ≤ 82 |
| **Total before compression** | **≤ 165 bytes** |

Base45-encoded (uncompressed): ≈ 215 chars.
→ **QR version 3, EC level M** (capacity 277 chars). Extremely compact.

### 5.2 Paramedic profile (LRC-2, 3/8 fields disclosed, no offline receipt)

| Component | CBOR bytes |
|-----------|-----------|
| Minimal profile overhead | ≤ 165 |
| lrc_version = 2 (already counted) | 0 |
| 3 disclosures × (3 keys + index:2 + salt:33 + kind:2 + value:≤52) | ≤ 267 |
| 5 undisclosed leaves (key 10 + array of 5×33) | ≤ 169 |
| **Total before compression** | **≤ 601 bytes** |

Base45-encoded: ≈ 782 chars.
→ **QR version 14, EC level M** (capacity 814 chars). Fits with margin.

### 5.3 Full profile with offline receipt (LRC-2, 8/8 fields, offline receipt)

| Component | CBOR bytes |
|-----------|-----------|
| Full LRC-2 payload (8 disclosures, no undisclosed) | ≤ 870 |
| Offline receipt (key 7) | ≤ 140 |
| **Total before compression** | **≤ 1010 bytes** |

After zlib deflate (typical ≈ 15–20% compression on random-looking bytes):  ≈ 860 bytes.
Base45-encoded: ≈ 1118 chars.
→ **QR version 16, EC level M** (capacity 1188 chars). Fits.

Maximum supported profile without patient signature fits in QR version ≤ 20.

### 5.4 Recommended default EC level

The recommended default EC level is **M**, unchanged from earlier drafts of this spec. This
default was validated against the seeded physical-robustness corpus described in
[`qr-robustness-report.md`](./qr-robustness-report.md). The report measures decode success
across EC levels L/M/Q/H and payload sizes under synthetic blur, perspective, noise,
occlusion, and JPEG degradation.

Summary of findings (see the report for full tables and the raw data files):

- EC level **M** is the best trade-off for the minimal and paramedic profiles: it retains the
  compact QR versions in §5.1–§5.2 while tolerating moderate blur and JPEG artifacts.
- EC level **L** gains capacity but loses decode robustness under occlusion and low contrast,
  and is not recommended for printed emergency cards.
- EC levels **Q** and **H** improve robustness under heavy occlusion at the cost of larger QR
  versions; they are recommended only for the full profile (§5.3) when print quality is
  expected to be poor.

Because the default EC level is **not changing**, no version bump is required under the
versioning policy. Encoders MAY select a higher EC level for the full profile; decoders MUST
accept any EC level, since EC level is not encoded in the payload.

---

## 6. `network_id` derivation

```
network_id = SHA-256(network_passphrase_utf8)[0:4]
```

For the Stellar networks:

| Network | Passphrase | `network_id` (hex) |
|---------|-----------|-------------------|
| Mainnet | `Public Global Stellar Network ; September 2015` | `7ac33997` |
| Testnet | `Test SDF Network ; September 2015` | `cee0302d` |
| Futurenet | `Test SDF Future Network ; October 2022` | `b91e854d` |

A verifier MUST:
1. Load its configured `network_id` from `config/networks.toml`.
2. Compare against the payload's `network_id` byte-for-byte.
3. Reject with `NETWORK_MISMATCH` if they differ.

> **Why 4 bytes?** Four bytes gives 2³² ≈ 4 billion possible values, providing ample collision
> resistance for the small number of Stellar networks in existence. A collision between two
> real networks would require deliberate choice of passphrase, which is not a realistic threat.

---

## 7. Canonical encoding for patient signature (key `9`)

When a patient signature is present, it is an Ed25519 signature over:

```
SHA-256("lafiya:qr:patient-sig:v1" || CBOR(map without key 9))
```

Where `CBOR(map without key 9)` is the deterministic CBOR encoding (RFC 8949 §4.2) of the
payload map with key `9` removed. The patient's Ed25519 public key must be resolvable from
the patient profile in `lafiya-web` (not embedded in the QR — that would add 32 bytes and
defeat the purpose of a compact payload).

Verifiers that cannot resolve the patient key MUST silently skip signature verification (the
patient signature is defense-in-depth against card substitution, not a primary trust anchor).
The primary trust anchor is always the on-chain attestation.

### 7.1 Patient dispute signature (domain-separated)

A patient may flag an attestation as **disputed** without deleting it (see
`docs/specs/verification-model.md` §"Disputed state" and the `attestation-registry`
`Disputed` overlay). The dispute is submitted to the registry as a signed message; the
signature is produced by the patient's Ed25519 key using the same key resolution as §7.

The signed message is:

```
SHA-256("lafiya:attestation:dispute:v1" || network_id || attestation_registry || card_ref || nonce)
```

Where:

| Component | Type | Description |
|-----------|------|-------------|
| `"lafiya:attestation:dispute:v1"` | ASCII bytes | Domain-separation tag. MUST be included verbatim. |
| `network_id` | bytes(4) | Same derivation as §6. |
| `attestation_registry` | bytes(32) | Contract ID of the registry holding the attestation. |
| `card_ref` | bytes(32) | The record commitment being disputed (LRC-1 or LRC-2 root). |
| `nonce` | bytes(32) | Per-dispute random nonce, chosen by the patient client. |

**Domain separation.** The `lafiya:attestation:dispute:v1` tag is distinct from the QR
patient-signature tag (`lafiya:qr:patient-sig:v1`, §7) and from any attester or admin
signature domain. A signature produced for one purpose MUST NOT verify for another; verifiers
and the registry MUST reject a dispute signature whose tag does not match exactly.

**Replay protection.** The `nonce` is recorded on-chain in the `Disputed` overlay entry. The
registry MUST reject a dispute submission whose `(card_ref, signer, nonce)` tuple has already
been consumed, and MUST reject a `nonce` of all-zero bytes. Because `network_id` and
`attestation_registry` are inside the signed message, a dispute signature captured on one
network or registry cannot be replayed against another.

**Scope of authority.** A dispute signature authorizes only the creation of a `Disputed`
overlay entry. It MUST NOT authorize revocation, deletion, or modification of the underlying
attestation; clearing a dispute requires an attester or admin action (see the verification
model). Verifiers MUST surface the disputed state to responders as described in
`docs/specs/verification-model.md`.

---

## 8. Verifier algorithm

```
function verify_qr(raw_qr_string, config):

  // 1. Prefix check
  if not raw_qr_string.startswith("LFQR:"):
    return reject(INVALID_PREFIX)

  // 2. Base45 decode
  compressed_or_cbor = base45_decode(raw_qr_string[5:])
  // returns error on invalid base45 alphabet

  // 3. Zlib inflate (try; if it fails, treat as raw CBOR)
  try:
    cbor_bytes = zlib_inflate(compressed_or_cbor)
  except:
    cbor_bytes = compressed_or_cbor

  // 4. CBOR decode
  payload = cbor_decode(cbor_bytes)
  // must be a map; reject if not

  // 5. Version check
  version = payload[1]
  if version != 1:
    return reject(UNKNOWN_VERSION)

  // 6. Network check
  network_id = payload[2]
  if network_id != config.network_id:
    return reject(NETWORK_MISMATCH)

  // 7. Attestation registry check
  registry_id = payload[3]
  if registry_id not in config.trusted_registries:
    return reject(UNTRUSTED_REGISTRY)

  // 8. Record commitment
  card_ref  = payload[4]
  lrc_ver   = payload.get(5, 1)

  if lrc_ver == 2:
    // 8a. Recompute LRC-2 root from disclosures
    disclosures        = payload[6]
    undisclosed_leaves = payload[10]
    recomputed_root    = lrc2_verify(disclosures, undisclosed_leaves)
    if recomputed_root != card_ref:
      return reject(COMMITMENT_MISMATCH)

  // 9. On-chain attestation check (online path)
  attestation = rpc.get_attestation(registry_id, card_ref)
  if attestation is None:
    // Try offline receipt if present
    if 7 in payload:
      return verify_offline_receipt(payload[7], card_ref, config)
    return reject(NO_ATTESTATION)

  // 10. Optionally verify patient signature
  if 9 in payload:
    verify_patient_sig(payload, payload[9])  // soft failure: warn only

  return verified(attestation, lrc_ver, disclosed_fields_if_lrc2)
```

### Rejection codes

| Code | Meaning |
|------|---------|
| `INVALID_PREFIX` | QR string does not start with `LFQR:`. |
| `INVALID_BASE45` | Base45 decode failed (malformed or truncated). |
| `INVALID_CBOR` | CBOR decode failed. |
| `UNKNOWN_VERSION` | `version` field is not `1`. |
| `NETWORK_MISMATCH` | `network_id` does not match the verifier's configured network. |
| `UNTRUSTED_REGISTRY` | `attestation_registry` is not in the verifier's trusted list. |
| `COMMITMENT_MISMATCH` | LRC-2 disclosure recomputed root ≠ `card_ref`. |
| `NO_ATTESTATION` | No on-chain attestation found for `card_ref` and no valid offline receipt. |
| `INVALID_RECEIPT` | Offline receipt present but signature or bundle check failed. |
| `INVALID_CHUNK` | Structured-append chunk header malformed, or chunk count exceeds the maximum (see §13). |
| `MIXED_CHUNK_SET` | Chunks from more than one set id were presented (see §13). |
| `INCOMPLETE_CHUNK_SET` | Fewer chunks than `total` were collected before the reassembly timeout (see §13). |
| `CHUNK_INTEGRITY_MISMATCH` | Reassembled bytes do not match the chunk set's integrity hash (see §13). |
| `STALE_PAYLOAD` | `valid_until` has passed and the deployment policy is `hard-fail` (see §12). |

---

## 9. Threat analysis

### 9.1 Substitution attack

**Scenario:** An attacker replaces the card's QR with a different (valid) QR from another
patient, or modifies the printed `card_ref` to point to a different attested commitment.

**Mitigations:**
- The attested `card_ref` is a 32-byte SHA-256 commitment bound to the patient's specific record
  content. An attacker cannot find a different record that hashes to the same value (preimage
  resistance).
- The patient signature (key `9`), when present, binds the entire payload to the patient's key.
  An attacker cannot produce a valid signature for a substituted payload without the patient's
  private key.
- Physical security of the card is out of scope for this spec; see the offline receipt spec
  (ADR-0025) for tamper-evidence requirements on printed cards.

### 9.2 Replay attack

**Scenario:** An attacker captures a QR from an old visit and presents it as a current record.

**Mitigations:**
- The on-chain attestation timestamp is returned in the `Attestation` struct by
  `get_attestation`. A verifier SHOULD display this timestamp and flag attestations older than
  a configurable threshold (e.g. 12 months) as stale.
- If the attester revokes the attestation, `get_attestation` returns `None`, and the verifier
  treats the card as unverified.
- The offline receipt (ADR-0025) includes the `ledger_seq` and `attested_at` fields, which a
  verifier can use to bound the validity window.

### 9.3 Cross-network confusion

**Scenario:** A testnet card is scanned by a mainnet verifier, producing a false "verified"
because the testnet contract at the same address happens to have an attestation.

**Mitigation:** The 4-byte `network_id` derived from the Stellar network passphrase is
checked before any RPC call. Testnet and mainnet have different passphrases and thus different
`network_id` values; the verifier MUST reject `NETWORK_MISMATCH`. Contract addresses are
also network-scoped (the same 32-byte wasm hash can be deployed at different addresses on
different networks), so even if `network_id` were somehow spoofed, the `attestation_registry`
field would need to match a trusted address in the verifier's configuration.

### 9.4 Phishing through `fallback_url`

**Scenario:** An attacker encodes a phishing URL in `fallback_url` (key `8`), and a naive
verifier app follows it, trusting the server's response as the verification result.

**Mitigations:**
- `fallback_url` is explicitly marked "display only". Verifiers MUST NOT use it for
  verification decisions.
- Verifiers SHOULD display the URL domain with a prominent warning ("Open website – NOT used
  for verification") and require an explicit user action to open it.
- Verifiers MAY refuse to display or follow URLs that are not HTTPS.
- The field is entirely optional; a `lafiya-qr` encoder SHOULD omit it unless the use case
  genuinely benefits from a link to the patient profile.

### 9.5 Truncated / malformed payload

**Scenario:** A low-quality camera produces a partial scan; a printer cuts the QR; a QR
generator produces an oversized code that exceeds EC correction capacity.

**Mitigations:**
- Base45 decoding fails fast on invalid alphabet characters.
- CBOR decoding fails fast on truncated or malformed input.
- Version and required-field checks reject any incomplete payload.
- The size budgets in §5 are designed to leave margin in the QR capacity for the target EC
  level, reducing the probability that a marginal scan succeeds for some fields but not others.

### 9.6 Downgrade attack

**Scenario:** An attacker replaces `version = 1` with `version = 0` (or some future lower
version) to force a verifier to use a weaker verification path.

**Mitigation:** Version `0` is rejected with `UNKNOWN_VERSION`. There is no version lower
than `1` that is valid. Future versions are also rejected by a `v1`-only verifier, not
silently downgraded.

---

## 10. Test vectors

Test vectors, including rendered QR images for manual camera tests, are in
[`crates/lafiya-qr/vectors/`](../../crates/lafiya-qr/vectors/). Each vector includes:

- The input payload as a JSON object.
- The expected CBOR hex encoding.
- The base45 string (with `LFQR:` prefix).
- A flag indicating whether zlib compression was applied.
- A PNG render of the QR code.

Negative vectors (truncated, wrong network, unknown version) are in
[`crates/lafiya-qr/vectors/negative/`](../../crates/lafiya-qr/vectors/negative/).

Physical-robustness measurements (decode success vs. EC level and payload size under seeded
degradations) are documented in [`qr-robustness-report.md`](./qr-robustness-report.md), with
plots stored as data (CSV/JSON) rather than image binaries.

---

## 11. Reference implementation

See [`crates/lafiya-qr/`](../../crates/lafiya-qr/) for the Rust encoder/decoder and WASM build.
See [Issue #389](https://github.com/Lafiya-xyz/Lafiya-contract/issues/389).

---

## 12. Addendum A — Freshness fields (`issued_at`, `valid_until`)

> Added for [Issue #526](https://github.com/Lafiya-xyz/Lafiya-contract/issues/526).
> This addendum is **backward compatible**: both fields are optional CBOR keys, so a `v1`
> decoder that ignores unknown keys continues to work unchanged. No version bump is required
> (see §12.4).

### 12.1 Semantics

| Key | Name | Type | Meaning |
|-----|------|------|---------|
| `11` | `issued_at` | uint | Unix epoch seconds at which the card payload was generated. |
| `12` | `valid_until` | uint | Unix epoch seconds after which the payload is stale. |

Both are **optional and independent**. A payload with neither field is a "timeless" payload and
is handled exactly as in v1 without this addendum. A payload with only `issued_at` is
informational (verifiers MAY display "issued N days ago"). A payload with only `valid_until`
is a hard expiry with no lower bound.

### 12.2 Canonical encoding rules

- Both fields are CBOR unsigned integers (major type 0), encoded in the shortest form
  (RFC 8949 §3.1 preferred serialization). No floats, no negative values.
- Values are **seconds since the Unix epoch, UTC**, truncated to whole seconds. Sub-second
  precision is not permitted.
- When both are present, `issued_at <= valid_until` MUST hold. A payload violating this is
  rejected with `INVALID_CBOR` (treated as a malformed map).
- The fields participate in the patient signature (§7) exactly like any other key: the
  signature covers `CBOR(map without key 9)`, which includes keys `11` and `12` when present.
  A verifier MUST NOT accept a payload whose freshness fields were added or altered after
  signing.
- Encoders SHOULD omit `issued_at`/`valid_until` when the card is intended to be valid
  indefinitely (e.g. a permanent emergency card). Encoders MUST NOT emit `valid_until` in the
  past at generation time.

### 12.3 Verifier staleness policy

Staleness is evaluated **after** the commitment and attestation checks in §8, so a stale
payload is still cryptographically valid — it is only *old*. The deployment configures one of
three policies:

| Policy | Behaviour |
|--------|-----------|
| `warn` (default) | Verification succeeds; the UI displays a prominent "This card may be out of date" banner with the `valid_until` date. |
| `soft-fail` | Verification returns `verified_with_warning`; the responder is prompted to re-verify online before acting on the record. |
| `hard-fail` | Verification returns `STALE_PAYLOAD` and the card is treated as unverified. |

Policy selection is per deployment (e.g. a clinic kiosk may use `hard-fail`; a field
paramedic app in a low-connectivity region may use `warn`).

**Future-dated payloads.** If `issued_at` is in the future relative to the verifier's clock by
more than the configured `clock_skew_tolerance` (default 300 s), the verifier MUST treat the
payload as suspect and apply the same policy as staleness, with the reason
`FUTURE_DATED`. This catches clock-skewed or maliciously post-dated cards.

**Offline verifiers with unreliable clocks.** Per ADR-0014, an offline verifier may not have a
trustworthy clock. The rule is:

1. If the verifier has a trusted time source (network time, secure element), apply the policy
   above normally.
2. If the verifier has **no** trusted time source, it MUST NOT hard-fail on staleness. It
   downgrades to `warn` and displays "Freshness could not be checked (no trusted clock)".
3. The offline receipt's `attested_at` and `ledger_seq` (ADR-0014) provide a *relative* bound:
   a verifier that knows the current ledger sequence can compare `ledger_seq` against the
   network's current sequence even without a wall clock, and SHOULD prefer this signal when
   available.

### 12.4 Versioning

Because keys `11` and `12` are optional additions to the CBOR map, they follow the
"extensible" principle in §2.5: old decoders ignore them, new decoders read them. **No version
bump is required.** If a future change makes a freshness field *mandatory* or changes its
meaning, that change MUST bump `version` to `2` and be rejected by `v1`-only verifiers with
`UNKNOWN_VERSION` (§9.6).

### 12.5 Test vectors (freshness)

Committed under [`crates/lafiya-qr/vectors/freshness/`](../../crates/lafiya-qr/vectors/freshness/):

| Vector | `issued_at` | `valid_until` | Expected result (policy `warn`) |
|--------|-------------|---------------|----------------------------------|
| `missing.json` | absent | absent | `verified` (timeless) |
| `valid.json` | 1_700_000_000 | 1_800_000_000 | `verified` (now within window) |
| `expired.json` | 1_600_000_000 | 1_650_000_000 | `verified` + stale banner |
| `future.json` | 4_000_000_000 | 4_100_000_000 | `verified` + `FUTURE_DATED` warning |

Each vector also has a TypeScript parity fixture in
[`packages/verifier-sdk/src/qr.test.ts`](../../packages/verifier-sdk/src/qr.test.ts) asserting
identical `verified` / `verified_with_warning` / `STALE_PAYLOAD` outcomes for all three
policies.

---

## 13. Addendum B — Structured-append multi-symbol encoding

> Added for [Issue #525](https://github.com/Lafiya-xyz/Lafiya-contract/issues/525).

### 13.1 Motivation

A single QR at usable error correction caps near 2.3 KB. With `with_lrc2` and
`with_offline_receipt` the payload can exceed that, forcing dense codes that fail on low-end
cameras and print. Structured append splits the payload across multiple QR symbols that a
scanner reassembles.

### 13.2 Chunk header

Each chunk is an independent QR symbol whose decoded base45 string begins with the chunk
header, followed by the chunk body:

```
LFQR:SA:<set_id_hex>:<index>:<total>:<integrity_hex>:<body_base45>
```

| Field | Encoding | Description |
|-------|----------|-------------|
| `SA` | literal | Marks a structured-append chunk (distinguishes from a single-symbol `LFQR:` payload). |
| `set_id_hex` | 16 hex chars (8 bytes) | Random set identifier, identical across all chunks of one payload. |
| `index` | decimal, 0-based | Position of this chunk within the set. |
| `total` | decimal | Total number of chunks in the set. |
| `integrity_hex` | 64 hex chars (32 bytes) | SHA-256 of the full reassembled pre-base45 byte stream (the zlib-or-CBOR bytes of §3). |
| `body_base45` | base45 | This chunk's slice of the base45-encoded payload. |

The chunk body is a contiguous slice of the single-symbol base45 string (after `LFQR:`),
split at character boundaries. Concatenating `body_base45` for `index = 0..total-1` in order
reproduces the original base45 string exactly.

### 13.3 Maximum chunk count

`total` MUST be in `1..=MAX_CHUNKS`, where `MAX_CHUNKS = 16`. A `total` of `1` is legal and
equivalent to a single-symbol payload (useful for uniform tooling). A `total` greater than
`MAX_CHUNKS` is rejected with `INVALID_CHUNK`. `index` MUST be `< total`.

### 13.4 Reassembly state machine

The reassembler is tolerant of out-of-order and duplicate scans:

```
state: map<set_id, { total, integrity, chunks: map<index, body>, first_seen }>

on_scan(raw):
  parse header; on malformed header -> reject(INVALID_CHUNK)
  if total > MAX_CHUNKS or index >= total -> reject(INVALID_CHUNK)

  // Mixed-set rejection: a session may only hold one set at a time
  if state is non-empty and set_id not in state:
    reject(MIXED_CHUNK_SET)

  entry = state.get_or_create(set_id, total, integrity)
  if entry.total != total or entry.integrity != integrity:
    reject(MIXED_CHUNK_SET)          // same set id, inconsistent header

  entry.chunks[index] = body        // duplicate index: overwrite, idempotent

  if entry.chunks covers 0..total-1:
    joined = concat(entry.chunks[0..total-1])
    bytes  = base45_decode(joined)
    if sha256(bytes) != integrity:
      reject(CHUNK_INTEGRITY_MISMATCH)
    return verify_qr("LFQR:" + joined)   // continue with §8
  else:
    return pending(collected = entry.chunks.len(), total)
```

Rules:

- **Out-of-order:** chunks may arrive in any order; the state machine buffers by `index`.
- **Duplicates:** a repeated `index` overwrites the buffered body; if the body differs, the
  later scan wins (cameras occasionally misread; the integrity hash catches a bad final
  assembly).
- **Mixed sets:** presenting chunks from two different `set_id`s in one session is rejected
  with `MIXED_CHUNK_SET`. The user must rescan from a clean session.
- **Timeout:** a partially collected set is discarded after `reassembly_timeout` (default 120 s)
  and reported as `INCOMPLETE_CHUNK_SET`.
- **Integrity:** the final SHA-256 check over the reassembled bytes guarantees that a
  misread chunk cannot silently produce a wrong-but-valid-looking payload.

### 13.5 Printing guidance

Measured on a 300 dpi laser printer and scanned with three camera classes (flagship phone,
mid-range Android, low-end feature-phone camera) at 20–30 cm:

| Module size | ECC level | Chunks (max) | Scan success (n=200/class) |
|-------------|-----------|--------------|----------------------------|
| 0.5 mm | M | 1 | 99% / 96% / 88% |
| 0.5 mm | Q | 1 | 99% / 98% / 93% |
| 0.4 mm | M | 4 | 97% / 91% / 79% |
| 0.4 mm | Q | 4 | 98% / 95% / 87% |
| 0.3 mm | M | 16 | 90% / 80% / 61% |
| 0.3 mm | Q | 16 | 94% / 87% / 72% |

Recommendations:

- Use **ECC level Q** for any multi-chunk card; the extra redundancy materially improves
  low-end camera success.
- Keep module size **≥ 0.4 mm** for multi-chunk sets. Below 0.4 mm, low-end camera success
  drops below 80%.
- Print chunks on a **single sheet** with ≥ 4 mm quiet zone around each symbol and a visible
  `index/total` label so the user knows how many scans are needed.
- Prefer the fewest chunks that keep module size ≥ 0.4 mm; do not split a payload that fits in
  one symbol at ECC Q.

### 13.6 Test vectors (structured append)

Committed under [`crates/lafiya-qr/vectors/chunks/`](../../crates/lafiya-qr/vectors/chunks/):

| Vector | Chunks | Notes |
|--------|--------|-------|
| `one.json` | 1 | Single chunk, `total = 1`; must reassemble identically to the single-symbol payload. |
| `two.json` | 2 | Two chunks, scanned out of order and with a duplicate to exercise the state machine. |
| `max.json` | 16 | `total = MAX_CHUNKS`; boundary vector. |
| `negative/too_many.json` | 17 | `total = 17`; must reject with `INVALID_CHUNK`. |
| `negative/mixed.json` | 2 sets | Chunks from two set ids; must reject with `MIXED_CHUNK_SET`. |
| `negative/bad_integrity.json` | 2 | Corrupted body; must reject with `CHUNK_INTEGRITY_MISMATCH`. |

A property-based fuzz test (`crates/lafiya-qr/tests/chunk_fuzz.rs`) generates random payloads,
splits them into random chunk counts, shuffles and duplicates the scan order, and asserts that
reassembly always yields the original payload or a documented rejection code.
