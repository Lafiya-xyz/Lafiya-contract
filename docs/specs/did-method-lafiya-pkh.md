# Lafiya DID Method Specification — `did:pkh:stellar` (pre-alpha)

**Version:** 0.1 (pre-alpha)
**Status:** Draft
**Authors:** Lafiya core team
**Relates to:** [ADR-0013](../adr/0013-did-resolution-for-attesters.md)

---

## 1. Introduction

This document specifies how Lafiya maps Stellar attester addresses to W3C
Decentralised Identifiers (DIDs) and how those DIDs are resolved to DID
Documents.

The chosen method is **`did:pkh`** with the `eip155:stellar` chain namespace,
following the [PKH DID Method specification](https://github.com/w3c-ccg/did-pkh).
This method derives a DID directly from a public-key hash (a Stellar account
address), requiring no additional infrastructure.

> **Scope:** This spec covers attester DIDs only. Patient DIDs are out of scope
> for this document and for the contracts in this repository.

---

## 2. DID Syntax

```
did-pkh-stellar  = "did:pkh:eip155:stellar:" stellar-address
stellar-address  = 56 * base32-char   ; G... strkey, RFC 4648 without padding
```

**Example:**

```
did:pkh:eip155:stellar:GAHJJJKMOKYE4RVPZEWZTKH5FVI4PA3VL7GK2LFNUBSGBKM7SFGNUQ2
```

---

## 3. CRUD Operations

### 3.1 Create

A `did:pkh:stellar` DID is **implicitly created** when the attester's address is
added to the `attester-registry` contract via `add_attester` or
`add_attester_with_info`. No separate DID registration call is required.

The DID is deterministic: given a Stellar address, the DID is
`did:pkh:eip155:stellar:<address>`.

### 3.2 Resolve

Given `did:pkh:eip155:stellar:<address>`, a resolver:

1. Queries `attester-registry::is_attester(<address>)` on the Lafiya network.
2. Queries `attester-registry::get_attester_status(<address>)` for suspension
   state. If the attester was never added, this returns `None`.
3. Derives the Ed25519 public key from the Stellar address (last 32 bytes of the
   decoded strkey payload, after removing the 1-byte version prefix and 2-byte
   CRC16 checksum).
4. Returns a DID Document (see Section 4) with `deactivated: true` if either
   `is_attester` is `false` or `suspended` is `true`.

**Resolution metadata:**

```json
{
  "contentType": "application/did+ld+json",
  "deactivated": false
}
```

### 3.3 Update

There is no explicit update operation for `did:pkh`. The DID and its
`verificationMethod` are bound to the Stellar address and do not change unless
the attester is removed and re-added with a new address (key rotation, see §5).

Attester metadata changes (`update_attester_info`) do not affect the DID
Document.

### 3.4 Deactivate

A DID is **deactivated** when either:
- `attester-registry::is_attester(<address>)` returns `false`
  (attester was never added or has been removed), or
- `attester-registry::get_attester_status(<address>).suspended == true`.

The resolver sets `"deactivated": true` in the DID Document metadata in both
cases. Deactivated DIDs must not be accepted as valid credential issuers by
verifiers.

---

## 4. DID Document

```json
{
  "@context": [
    "https://www.w3.org/ns/did/v1",
    "https://w3id.org/security/suites/ed25519-2020/v1"
  ],
  "id": "did:pkh:eip155:stellar:<stellar-address>",
  "verificationMethod": [
    {
      "id": "did:pkh:eip155:stellar:<stellar-address>#key-1",
      "type": "Ed25519VerificationKey2020",
      "controller": "did:pkh:eip155:stellar:<stellar-address>",
      "publicKeyMultibase": "<multibase-encoded Ed25519 public key>"
    }
  ],
  "assertionMethod": [
    "did:pkh:eip155:stellar:<stellar-address>#key-1"
  ],
  "authentication": [
    "did:pkh:eip155:stellar:<stellar-address>#key-1"
  ]
}
```

**Key derivation from Stellar address:**

A Stellar `G...` address encodes a 32-byte Ed25519 public key. Decoding:

1. Base32-decode (RFC 4648, no padding) the `G...` string → 35 bytes.
2. Byte 0: version byte (`0x30` for account).
3. Bytes 1–32: Ed25519 public key.
4. Bytes 33–34: CRC16 checksum (verified, not included in the key).
5. Multibase-encode bytes 1–32 with prefix `z` (base58btc) to produce
   `publicKeyMultibase`.

---

## 5. Security Considerations

### Key compromise

`did:pkh` binds the DID to the Stellar address. If the attester's Stellar private
key is compromised:

- The attacker can sign VCs as that attester.
- The admin must call `remove_attester(<compromised-address>)` to deactivate the
  DID.
- The attester must generate a new Stellar keypair and be re-added with the new
  address.

This key-rotation gap (the window between compromise and deactivation) is an
accepted risk at pre-alpha scale. See ADR-0013 for the upgrade path to a method
supporting explicit key rotation.

### Suspension vs removal

- **Suspended** attesters have their DID deactivated (resolver returns
  `deactivated: true`) but remain in the registry. Reinstatement reactivates the
  DID.
- **Removed** attesters have their DID permanently deactivated for that address.
  Re-enrollment with the same address re-activates it.

---

## 6. Privacy Considerations

The DID Document contains only:
- The Stellar address (already public in the attester allowlist).
- The Ed25519 public key (derived from the address — also public).
- The `deactivated` flag (derived from on-chain state).

It does **not** contain the attester's real name, employer, license number, or
geographic region. The `license_hash` stored in `attester_info` is a hash of an
off-chain document — it is not included in the DID Document and does not reveal
any personal data.

---

## 7. Conformance

A conformant Lafiya DID resolver MUST:

- Accept DIDs matching `did:pkh:eip155:stellar:[A-Z2-7]{56}`.
- Query `attester-registry::is_attester` and `get_attester_status` on the
  network specified in the resolution options.
- Return `deactivated: true` for suspended or removed attesters.
- Return an error document for addresses that have never been added.
- Derive `publicKeyMultibase` from the Stellar address as specified in §4.
- Not include any off-chain metadata (name, employer, license number) in the
  DID Document.

---

## 8. Reference Implementation

A reference Rust resolver (`LafiyaDidResolver`) will be added to
`crates/lafiya-did/` as part of post-M1 work. It implements the
[`did-resolver`](https://docs.rs/did-resolver) crate trait and uses
`crates/lafiya-config` for network configuration.

A Universal Resolver HTTP driver wrapping `LafiyaDidResolver` will be published
in a separate repository under the `lafiya-xyz` GitHub organisation.

---

## 9. Test Vectors

The following test vectors cover the resolver's expected behaviour against all
attester states:

| Scenario | `is_attester` | `suspended` | Expected `deactivated` |
|---|---|---|---|
| Active attester | `true` | `false` | `false` |
| Suspended attester | `true` | `true` | `true` |
| Removed attester | `false` | n/a | `true` |
| Never added | `false` | n/a | `true` |

All four states are covered by unit tests in `crates/lafiya-did/src/tests.rs`
(post-M1).
