# lafiya-commitment

Prototype implementation for the record-commitment canonicalization spike
([issue #133](https://github.com/Lafiya-xyz/Lafiya-contract/issues/133)).
The full decision — comparison matrix, threat analysis, and migration plan —
is in [`docs/adr/0008-record-commitment-canonicalization.md`](../../docs/adr/0008-record-commitment-canonicalization.md).
This crate implements the Lafiya Record Commitment v1 (LRC-1) scheme that
ADR describes: a fixed-schema, length-prefixed binary encoding hashed with
SHA-256 under a domain-separation tag and an explicit version byte.

This is off-chain application code, not a Soroban contract. Per
[ADR-0001](../../docs/adr/0001-hash-only-on-chain-footprint.md), the
contracts only ever see the resulting opaque 32-byte commitment.

## Layout

- [`src/lib.rs`](src/lib.rs) — Rust implementation.
- [`reference-ts/lrc1.ts`](reference-ts/lrc1.ts) — TypeScript reference
  implementation, field-for-field and byte-for-byte identical to the Rust
  version. Illustrative of the construction the web application should
  follow; not a published package.
- [`vectors/lrc1-test-vectors.json`](vectors/lrc1-test-vectors.json) — the
  shared fixture both implementations are tested against.
- [`src/salt.rs`](src/salt.rs), [`reference-ts/salt.ts`](reference-ts/salt.ts),
  [`reference-py/salt.py`](reference-py/salt.py) — deterministic salt
  derivation (HKDF-SHA256 from a patient master secret,
  [ADR-0018](../../docs/adr/0018-deterministic-salt-derivation.md)), checked
  against [`vectors/salt-derivation-vectors.json`](vectors/salt-derivation-vectors.json).
- [`src/alg.rs`](src/alg.rs) — reserved cryptographic algorithm identifiers
  ([ADR-0019](../../docs/adr/0019-cryptographic-agility.md)).

## Running the tests

Rust:

```sh
cargo test -p lafiya-commitment
```

TypeScript (Node 22+; no build step or dependency — Node strips the type
annotations natively):

```sh
node crates/lafiya-commitment/reference-ts/lrc1.test.ts
```

Salt derivation (TypeScript and Python; the Rust vectors run with
`cargo test`):

```sh
node crates/lafiya-commitment/reference-ts/salt.test.ts
python3 crates/lafiya-commitment/reference-py/salt_test.py
```

CI runs all of these in the `commitment-vectors` job.

The LRC-1 suites load `vectors/lrc1-test-vectors.json` and assert their output
matches it byte-for-byte, which is the cross-language determinism the spike
was asked to demonstrate.

## Security notes

Salts and the pre-hash record fields are secret: anyone holding them can
run a dictionary attack against the on-chain commitment. The crate handles
them as follows.

- **Zeroization.** `Salt` overwrites its 32 bytes with zeros on drop and
  implements `Zeroize`/`ZeroizeOnDrop`. The salted preimage built by
  `commit_v1_salted` (salt plus encoded fields) lives in a `Zeroizing`
  buffer that is allocated at its exact final size, so it never reallocates
  and leaves stale copies in freed memory. The WASM `CommitmentEncoder`
  wipes the `Uint8Array` copy it receives from JS and its own stack copy.
- **No accidental copies or logging.** `Salt` implements neither `Clone`,
  `Copy`, `Debug` nor `Display`. A compile-time guard in `src/lib.rs`
  (`secret_type_guards`) makes the crate, and therefore `cargo clippy`, fail
  if any of those are ever derived for it.
- **Constant-time comparison.** Use `verify_v1_salted` (or
  `commitments_eq` for two raw commitments) instead of `==` when one side
  was recomputed from secret inputs. `Salt` only offers equality through
  `subtle::ConstantTimeEq`. The LRC-2 `verify_disclosure` root check uses
  the same constant-time comparison.

### Limits of these guarantees

Zeroization is best effort, not a guarantee that no copy ever exists:

- `Salt::from_bytes` takes its array by value; the caller's copy, and any
  temporaries the compiler spills to the stack or registers, are outside
  this crate's control.
- `FieldValue` is an ordinary owned value (`Clone`, `Debug`) because it is
  also used for non-secret data. Callers holding sensitive fields should
  wrap them in `zeroize::Zeroizing` themselves.
- The SHA-256 implementation (`sha2`) does not zeroize its internal block
  buffer, and the Soroban `commit_v1_soroban_salted` path copies the
  preimage into host-managed `Bytes`, which this crate cannot wipe.
- In WASM, linear memory is a JS `ArrayBuffer`: anything with a reference to
  the module's memory can read it, and the original `Uint8Array` the caller
  passed in stays in the JS heap until the caller overwrites it
  (`salt.fill(0)`). Call `encoder.free()` explicitly; relying on the
  `FinalizationRegistry` defers zeroization indefinitely.
- Constant-time behavior is provided by `subtle`'s optimization barriers.
  It holds for the comparison itself, not for SHA-256 or the encoding,
  whose running time depends on the (public) field lengths. JIT-compiled
  WASM gives no formal constant-time guarantee; `subtle` is the best
  available mitigation there.
