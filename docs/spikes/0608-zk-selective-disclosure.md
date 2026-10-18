# Spike: Zero-Knowledge Selective Disclosure for Emergency Cards

Tracks [#608](https://github.com/Lafiya-xyz/Lafiya-contract/issues/608). Baseline:
[ADR-0012 LRC-2 selective disclosure](../adr/0012-lrc2-selective-disclosure.md).

## Question

Can ZK proofs supersede per-field Merkle disclosure, proving predicates (e.g. "not allergic
to penicillin") without revealing field values or positions?

## Primitives available to Soroban

| Primitive | Status | Notes |
| --- | --- | --- |
| BLS12-381 host functions (G1/G2 add, mul, multi-scalar-mul, pairing) | Available (Protocol 22+) | Enough to verify Groth16 / PLONK-KZG proofs in a contract. |
| BN254 host functions | Check current protocol | Cheaper EVM-style Groth16 if available; confirm against the deployed protocol version. |
| Poseidon / Poseidon2 host functions | Check current protocol | Needed so in-circuit hashing matches on-chain commitments cheaply. |
| Pure-WASM verification | Always possible | Cost dominated by pairings; generally exceeds per-tx budgets. |

Versions and availability change; re-confirm against the Soroban release notes when running
the measurements below.

## Prototype plan (reproducible)

Predicate: *"the committed `allergies` list does not contain `penicillin`"* over the LRC-2
Merkle root.

1. Circuit (circom or noir): inputs are the Merkle root (public), predicate id (public), the
   field list and Merkle paths (private). Output: a Groth16 proof over BLS12-381.
2. Verifier contract: a scratch contract (not merged to `contracts/`) calling the pairing
   host functions.
3. Measure with `stellar contract invoke --sim`: proof size (bytes), CPU instructions, memory,
   ledger-entry reads, and fee at current network settings. Record results in the table below.
4. Measure prover time on a low-end Android device, since proving happens on the patient side.

| Metric | Merkle (LRC-2) | Groth16 prototype |
| --- | --- | --- |
| Proof/disclosure size | _to be measured_ | _to be measured_ |
| Verification CPU instructions | _to be measured_ | _to be measured_ |
| Verification fee | _to be measured_ | _to be measured_ |
| Prover time (low-end device) | n/a | _to be measured_ |
| Trusted setup | none | per-circuit ceremony required |

> **Status:** the survey and plan are written; the measurement table is **not yet filled**.
> No numbers are asserted here until they are produced by the steps above.

## Recommendation: defer, with adoption triggers

Defer. Merkle disclosure (ADR-0012) meets today's requirements, has no trusted setup, and
verifies offline on low-end devices. Revisit when **all** of these hold:

1. Measured on-chain verification fits within a single-transaction budget with headroom.
2. A product requirement exists for predicate proofs that Merkle disclosure cannot meet (hiding
   field position or proving absence).
3. Proving completes within an agreed time on the reference low-end Android device.
4. A circuit audit and setup-ceremony plan are funded.

## Follow-ups if adopted

- ADR for the proof system and circuit versioning.
- Verifier contract + conformance tests.
- Wallet-side prover integration in `lafiya-web`.
