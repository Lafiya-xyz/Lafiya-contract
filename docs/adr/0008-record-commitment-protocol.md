# ADR-0008: Define a Versioned, Domain-Separated Record Hashing Protocol

- **Status:** Accepted
- **Date:** 2026-08-17
- **Deciders:** Lafiya contract maintainers

## Context

Lafiya health records live off-chain. Opaque 32-byte commitments are saved on-chain. To support this:
1. **Deterministic Serialization**: If different clients serialize identical emergency records with different key ordering or whitespace, they will produce different hash values. This breaks verification.
2. **Domain Separation**: Hash values must be uniquely tied to Lafiya emergency records so that a hash cannot be reused or replayed from another cryptographic context.
3. **Entropy & Hiding (Salt)**: Many critical health fields (e.g., blood group like `O+`, genotype like `AA`) have very low entropy. Without random salt, an observer could brute-force guess the patient's private medical details by comparing the on-chain hash against precomputed hashes of all possible value combinations.

## Decision

We define a versioned, domain-separated record hashing protocol for `Lafiya-contract`.

### 1. Data Model
An emergency record JSON document must contain exactly the following fields (even if empty):
- `blood_group` (String or null): The patient's blood group (e.g. `"O+"`, `"A-"`, `null`).
- `genotype` (String or null): The patient's genotype (e.g. `"AA"`, `"AS"`, `"SS"`, `null`).
- `allergies` (Array of Strings): Known allergies (e.g., `["penicillin"]`).
- `current_medications` (Array of Strings): Active medications.
- `chronic_conditions` (Array of Strings): Active chronic conditions.
- `salt` (String): A 32-character hexadecimal string representing 16 cryptographically secure random bytes.

### 2. Canonical Serialization
The JSON document must be serialized using **RFC 8785: JSON Canonicalization Scheme (JCS)**. JCS enforces:
- Key ordering: Keys are sorted lexicographically by UTF-16 code units.
- Minimal whitespace: No indentation, newlines, or extra spaces.
- Consistent character escaping.

### 3. Domain Separation
To prevent replay attacks across different contexts, we prepend a Domain Separation Tag (DST):
- **DST**: `"Lafiya-Emergency-Record-v1\0"` (a null-terminated UTF-8 byte array of 27 bytes: `Lafiya-Emergency-Record-v1` followed by a `0x00` byte).

### 4. Hash Algorithm
The final commitment hash is calculated using the standard SHA-256 algorithm:
$$\text{Commitment Hash} = \text{SHA-256}(\text{DST} \parallel \text{JCS}(\text{Record JSON}))$$

## Alternatives considered

### CBOR or Protocol Buffers
Rejected due to implementation and dependency overhead on the patient-facing client side (`lafiya-web`) and developer experience. JSON is universally supported and JCS ensures determinism with minimal overhead.

### Plain SHA-256 Hashing without Domain Separation or Salt
Rejected because it allows cross-context replay of hashes and exposes sensitive fields to dictionary brute-forcing attacks.

## Consequences

### Positive
- Different clients can reliably compute identical hashes for identical records.
- Patient health privacy is robustly protected against brute-force dictionary attacks.
- Commitments are bound to the specific versioned context of Lafiya emergency records.

### Trade-offs and risks
- The patient's profile store must securely store the generated `salt`. If the `salt` is lost, the card cannot be re-verified against the on-chain registry.
- Standard JSON stringifiers do not sort keys automatically, requiring helper canonicalization methods in both Rust and TypeScript implementations.

## References

- [ADR-0001](0001-hash-only-on-chain-footprint.md)
- [RFC 8785 (JCS)](https://datatracker.ietf.org/doc/html/rfc8785)
