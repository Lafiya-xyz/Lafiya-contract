# Kani bounded proofs for codecs

Kani harnesses live next to the code (`#[cfg(kani)] mod kani_proofs`) in:

- `crates/lafiya-commitment/src/lib.rs`: LRC-1 canonical payload encoding
- `crates/lafiya-qr/src/base45.rs`: base45 (RFC 9285)
- `crates/lafiya-qr/src/cbor.rs`: minimal CBOR subset

Run locally (Linux/macOS; Kani does not support Windows): `cargo install --locked kani-verifier && cargo kani setup`, then `cargo kani` inside each crate. CI: `.github/workflows/kani.yml` (20-minute job cap).

| Harness | Property | Bound |
| --- | --- | --- |
| `commitment::encode_decode_roundtrip` | `decode_payload(encode_payload(x)) == x` | ≤ 2 fields; bytes/text ≤ 3 bytes; text ASCII |
| `commitment::decode_never_panics` | `decode_payload` does not panic | ≤ 6 input bytes |
| `base45::roundtrip` | `decode(encode(x)) == x` | ≤ 3 bytes |
| `base45::decode_never_panics` | `decode` does not panic | ≤ 6 ASCII chars |
| `cbor::scalar_roundtrip` | encode/decode of `Uint`, negative `Int`, `Bool` is lossless and consumes all bytes | full `u64`/`i64` range |
| `cbor::decode_never_panics` | `decode_value` does not panic | ≤ 5 input bytes |

## Not covered (soundness caveats)

- **Bounded only.** Results hold for the sizes above. Loop-length, size-limit paths (`MAX_FIELD_BYTES`, `MAX_FIELDS`, `MAX_PAYLOAD_BYTES`) and multi-byte-chunk interactions beyond the bounds are covered by fuzzing and unit tests, not proofs.
- **No hashing.** `commit_v1*` SHA-256 is not modelled; the proofs cover the canonical payload that feeds it. Injectivity of the encoding (hence commitment binding) follows from the round-trip only for in-bound inputs.
- **Text is ASCII-only** in harnesses; non-ASCII UTF-8 and NFC normalization are not covered.
- **CBOR:** arrays, maps, byte/text strings and nesting are not proven (unbounded recursion); only scalars and decoder panic-freedom on short input. Positive `Int` is out of scope because the decoder canonicalizes it to `Uint`.
- **Allocation failure and overflow** follow Kani's default checks; stack depth and timing are not modelled.
- Unwind bounds are chosen to be exhaustive for the input bounds; Kani reports an unwinding failure rather than silently truncating.

If a harness reveals a bug, fix it in the codec and add a regression unit test in the same PR.
