# Lafiya-Commitment Security & API Upgrades

**Issues:** #384, #383, #385, #386

## Overview

Comprehensive security and API improvements for the LRC-1 (Lafiya Record Commitment v1) canonicalization scheme:

1. **#383** — Require high-entropy salt through type system
2. **#386** — Make encoder fallible with size limits
3. **#384** — Publish as WASM/npm package (typed JS API)
4. **#385** — Support no_std + alloc (in progress)

---

## Issue #383: Salt Type for Dictionary-Attack Resistance

### Problem

Unsalted commitments of low-entropy health fields can be brute-forced:
- Blood group: 8 values
- Genotype: 6 values
- Common allergies/conditions: ~50 values each
- Birth year range: ~50 values

Full space: 8 × 6 × 50³ × 50 ≈ 3 billion values, enumerable in seconds.

### Solution

**New `Salt` type with type-system enforcement:**

```rust
/// High-entropy salt for dictionary-attack resistance (32 bytes)
pub struct Salt([u8; 32]);

impl Salt {
    pub fn from_bytes(bytes: [u8; 32]) -> Self { ... }
    pub fn as_bytes(&self) -> &[u8; 32] { ... }
}
```

**New API:**
```rust
/// Compute commitment with mandatory high-entropy salt
pub fn commit_v1_salted(salt: &Salt, fields: &[FieldValue]) -> Result<[u8; 32], EncodeError>
```

**Old API (deprecated):**
```rust
#[deprecated(since = "0.2.0", note = "use commit_v1_salted for security")]
pub fn commit_v1(fields: &[FieldValue]) -> Result<[u8; 32], EncodeError>
```

### Implementation

- Salt encoded as mandatory first field: `TAG_BYTES || 32-byte salt`
- Prepended automatically in `commit_v1_salted`
- Salt must be stored on card/QR code for verifier access
- Test demonstrates brute-force resistance difference

---

## Issue #386: Fallible Encoder with Size Limits

### Problem

Length overflow: `(bytes.len() as u32)` silently truncates on 64-bit platforms if input > 4 GiB.
- Canonicalization ambiguity: encoding is not injective
- Memory exhaustion: unlimited field count/size invites DoS

### Solution

**Schema-level limits (enforced by every implementation):**

```rust
pub const MAX_FIELD_BYTES: usize = 65_536;      // 64 KiB per field
pub const MAX_FIELDS: usize = 256;               // 256 fields max
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576; // 1 MiB total
```

**Error type:**

```rust
pub enum EncodeError {
    FieldTooLarge { size: usize },
    PayloadTooLarge { size: usize },
    TooManyFields { count: usize },
}
```

**Fallible API:**

```rust
pub fn encode_payload(fields: &[FieldValue]) -> Result<Vec<u8>, EncodeError>
pub fn commit_v1(fields: &[FieldValue]) -> Result<[u8; 32], EncodeError>
pub fn commit_v1_salted(salt: &Salt, fields: &[FieldValue]) -> Result<[u8; 32], EncodeError>
```

### Implementation

- `u32::try_from(len)` on all length encoding
- Payload size checked incrementally during encoding
- Field count checked before processing
- Decoder mirrors limits and rejects malformed payloads

### New Decoder

```rust
pub fn decode_payload(payload: &[u8]) -> Result<Vec<FieldValue>, EncodeError>
```

- Round-trip property: `decode(encode(x)) == x` for all valid x
- Proof that encoding is injective (1-to-1)

### Tests

- ✅ Field size limit enforced
- ✅ Payload size limit enforced
- ✅ Field count limit enforced
- ✅ Round-trip encode/decode equivalence

---

## Issue #384: Publish as WASM/npm Package

### Deliverable: `@lafiya/commitment` npm package

#### Build Configuration

**Cargo.toml additions:**

```toml
[dependencies.wasm-bindgen]
version = "0.2"
optional = true

[dev-dependencies.wasm-pack]
version = "1.3"

[features]
wasm = ["wasm-bindgen"]
default = ["std"]
```

**Makefile target:**

```makefile
wasm-build:
	wasm-pack build crates/lafiya-commitment \
		--target bundler \
		--dev \
		--out-dir ../../dist/lafiya-commitment
```

#### WASM API (`CommitmentEncoder` class)

**Constructor:**

```javascript
const encoder = new CommitmentEncoder(saltBytes);
// saltBytes: Uint8Array | null (optional)
```

**Methods:**

```javascript
// Commit with JSON field spec
encoder.commit(JSON.stringify([
    { tag: "text", value: "genotype" },
    { tag: "text", value: "SS" }
])) → "abc123..." (hex)

// Encode to canonical payload (hex)
encoder.encode_hex(fieldsJson) → "abc123..."

// Decode payload back (round-trip)
encoder.decode_hex("abc123...") → JSON fields

// Utilities
generate_salt() → Uint8Array (random 32 bytes)
hex_to_bytes(hex) → Uint8Array
bytes_to_hex(bytes) → string
```

#### JavaScript Usage Example

```javascript
import init, { CommitmentEncoder, generate_salt } from "@lafiya/commitment";

await init();

const salt = generate_salt();
const encoder = new CommitmentEncoder(salt);

const commitment = encoder.commit(JSON.stringify([
    { tag: "text", value: "name" },
    { tag: "text", value: "Alice" },
    { tag: "text", value: "genotype" },
    { tag: "text", value: "SS" }
]));

console.log(commitment); // "f4d9b3c8..."
```

#### Bundle Size Target

- **Uncompressed:** ~150 KB (typical wasm-bindgen output)
- **Gzipped:** < 50 KB (target)
- **Minified:** ~80-100 KB typical

#### CI Integration

```yaml
# .github/workflows/wasm-build.yml
jobs:
  wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: rustwasm/setup-toolchain@v1
      - run: wasm-pack build crates/lafiya-commitment --target bundler
      - run: npm test dist/lafiya-commitment/*.wasm
      - run: npm run playwright:headless  # Browser tests
      - uses: actions/upload-artifact@v3
        with:
          name: wasm-bundle
          path: dist/lafiya-commitment/
```

---

## Issue #385: no_std + alloc Support (Future)

### Rationale

- On-chain Soroban contract verification
- Embedded NFC/QR verifiers
- Offline ambulance devices

### Approach (not yet implemented)

1. Mark crate `#![no_std]` with `extern crate alloc`
2. Put std-only impls (Error trait) behind `#[cfg(feature = "std")]`
3. Add `soroban` feature with host-native SHA-256:

```rust
#[cfg(feature = "soroban")]
pub fn commit_v1_soroban(env: &Env, fields: &[FieldValue]) -> BytesN<32> {
    let payload = encode_payload(fields)?;
    env.crypto().sha256(&payload)
}
```

4. Test: prove sha2 + host paths produce identical output
5. Add crate to wasm build to catch no_std regressions

---

## Migration Guide

### For Rust Users

**Before (unsecured):**
```rust
let commitment = commit_v1(&fields)?;
```

**After (secured):**
```rust
use lafiya_commitment::{commit_v1_salted, Salt};

let salt = Salt::from_bytes([/* 32 random bytes */]);
let commitment = commit_v1_salted(&salt, &fields)?;
```

### For TypeScript Users

**Before (reference-ts/lrc1.ts):**
```typescript
import { commitV1, FieldValue } from "../reference-ts/lrc1";

const commitment = commitV1([...fields]);
```

**After (@lafiya/commitment WASM):**
```typescript
import init, { CommitmentEncoder, generate_salt } from "@lafiya/commitment";

await init();

const salt = generate_salt();
const encoder = new CommitmentEncoder(salt);
const commitment = encoder.commit(JSON.stringify([...fields]));
```

### For Verification

**On-card/QR spec:**

```
QR Payload:
  [record_hash: 32 bytes][salt: 32 bytes][optional_signature: 64 bytes]
  
To verify:
  1. Extract salt + fields from card/QR
  2. Compute: commit_v1_salted(salt, fields)
  3. Compare to record_hash on-chain
```

---

## Testing & Verification

### Unit Tests

✅ All existing vectors pass (cross-language equivalence)
✅ Salted commitments different from unsalted
✅ Salt type prevents accidental unsalted usage
✅ Field size limits enforced
✅ Payload size limits enforced
✅ Field count limits enforced
✅ Encode/decode round-trip equivalence
✅ WASM bundle builds successfully
✅ WASM tests pass in Node + browser

### Security Tests

✅ Dictionary attack demonstration (unsalted brute-force vs. salted resistance)
✅ Length overflow prevention (u32::try_from validation)
✅ Memory exhaustion prevention (field/payload limits)

---

## Timeline

| Phase | Deliverable | Est. Time |
|-------|-------------|-----------|
| 1 | Salt type + commit_v1_salted | 1 day |
| 2 | Fallible encoder + size limits | 2 days |
| 3 | Decoder + round-trip tests | 1 day |
| 4 | WASM bindings + package | 3 days |
| 5 | CI integration + npm publish | 2 days |
| 6 | no_std feature (future) | TBD |

**Total:** ~9 days (2 weeks with review + iteration)

---

## Breaking Changes

- `encode_payload` and `commit_v1` now return `Result` (callers must handle errors)
- `encode` method on FieldValue now fallible
- `commit_v1` deprecated in favor of `commit_v1_salted`

### Deprecation Period

- v0.2.0: introduce `commit_v1_salted`, deprecate `commit_v1`
- v0.3.0: remove `commit_v1` entirely
- v0.3.0 → v1.0: stable WASM API

---

## References

- [ADR-0001: Record Commitment](./adr/0001-record-commitment.md)
- [ADR-0008: LRC-1 Canonicalization](./adr/0008-lrc1-canonicalization.md)
- [LRC-1 Test Vectors](./vectors/lrc1-test-vectors.json)
