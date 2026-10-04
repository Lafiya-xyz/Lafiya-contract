# Lafiya QR Payload v1 Specification

- **Status:** Draft
- **Version:** 1 (`0x01`)
- **Date:** 2026-09-27
- **Authors:** Lafiya contract maintainers
- **Related:** [Issue #389](https://github.com/Lafiya-xyz/Lafiya-contract/issues/389),
  [ADR-0008 (LRC-1)](../adr/0008-record-commitment-canonicalization.md),
  [ADR-0012 (LRC-2)](../adr/0012-lrc2-selective-disclosure.md),
  [ADR-0014 (offline receipts)](../adr/0014-offline-attestation-receipts.md)

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
QR code (ISO/IEC 18004), alphanumeric mode, EC level M
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
| `7` | `offline_receipt` | map | No | Signed attestation receipt for offline verification (see ADR-0014). |
| `8` | `fallback_url` | tstr | No | Untrusted display-only URL. Verifiers MUST NOT use this for verification. |
| `9` | `patient_sig` | bytes(64) | No | Ed25519 signature by the patient's key over the canonical encoding of fields 1–5. |

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

See ADR-0014. When present, this is a CBOR map with at minimum:

| Sub-key | Name | Type |
|---------|------|------|
| `1` | `v` | uint (= 1) |
| `2` | `attester` | bytes(32) |
| `3` | `attested_at` | uint (Unix epoch seconds) |
| `4` | `ledger_seq` | uint |
| `5` | `attester_sig` | bytes(64) |

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
  (ADR-0014) for tamper-evidence requirements on printed cards.

### 9.2 Replay attack

**Scenario:** An attacker captures a QR from an old visit and presents it as a current record.

**Mitigations:**
- The on-chain attestation timestamp is returned in the `Attestation` struct by
  `get_attestation`. A verifier SHOULD display this timestamp and flag attestations older than
  a configurable threshold (e.g. 12 months) as stale.
- If the attester revokes the attestation, `get_attestation` returns `None`, and the verifier
  treats the card as unverified.
- The offline receipt (ADR-0014) includes the `ledger_seq` and `attested_at` fields, which a
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

---

## 11. Reference implementation

See [`crates/lafiya-qr/`](../../crates/lafiya-qr/) for the Rust encoder/decoder and WASM build.
See [Issue #389](https://github.com/Lafiya-xyz/Lafiya-contract/issues/389).
