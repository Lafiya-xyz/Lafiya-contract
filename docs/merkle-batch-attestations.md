# Merkle-batched attestations

`attestation-registry::anchor_batch(attester, root, leaf_count)` lets one
allowlisted attester authorize many record commitments with one transaction.
The contract stores one persistent `AttestationBatch` entry per root rather
than one attestation entry per record. The batch metadata exposes the
attester, anchoring ledger timestamp, and leaf count; the root is the lookup
key. A root cannot be anchored twice.

## Tree format

For a batch of 32-byte record commitments:

1. Hash each leaf as `SHA-256(0x00 || record_hash)`.
2. Hash each pair of child nodes as
   `SHA-256(0x01 || min(left, right) || max(left, right))`, where the
   comparison is lexicographic over the 32-byte values.
3. When a level has an odd number of nodes, pair the last node with itself.
   Repeat until one 32-byte root remains.

The `leaf_count` passed to `anchor_batch` must equal the number of commitments
used to construct the tree. The contract records this count but cannot infer
the off-chain leaves or validate the root's construction. A verifier must
check the inclusion proof against this format and the stored root, and must
use the anchored attester and timestamp as the claim context. As with a
single attestation, anchoring proves what an allowlisted attester authorized;
it does not prove the underlying medical data is correct.

Merkle anchoring does not replace `attest`. Use `attest` when per-record
on-chain lookup, history, and revocation are required. Batches can be
revoked as a whole or per leaf (see "Revocation" below).

## Attester namespacing (#659)

Batches are stored under `AttesterBatch(attester, root)`, and
`get_attestation_batch(attester, root)` is attester-scoped. A second attester
anchoring the same root creates its own entry; it can neither block nor
re-attribute the first attester's batch.

Leaves must commit to the anchoring attester:
`leaf = sha256(0x00 || "lafiya-batch-leaf-v1" || attester_xdr || record_hash)`.
Verifiers must reject an inclusion proof whose leaf does not commit to the
attester passed to `get_attestation_batch`.

**Compatibility:** batches anchored before this change remain under the legacy
`AttestationBatch(root)` key. `get_attestation_batch` falls back to it only
when the stored attester matches the queried attester, and `anchor_batch`
refuses to re-anchor a legacy root for the same attester. No migration is
required; legacy entries expire with their TTL.

## Policy parity with `attest` (#660)

| Guard on `attest` | Batch equivalent |
|---|---|
| `require_auth`, pause, allowlist | Same checks in `anchor_batch` |
| Rate limit (`consume_rate_limit`) | Charged `leaf_count` units in one call |
| Probation cap (`consume_probation_cap`) | Charged `leaf_count` units; probationary attesters may batch only within their cap |
| Size bound | `leaf_count <= MAX_BATCH_LEAVES` (1024), else `BatchTooLarge` |
| Patient consent | Enforced by verifiers: each leaf must be accompanied off-chain by a patient consent token (see `attest_with_consent_token`) bound to the same `record_hash` and attester; verifiers reject leaves without one |
| Version link / scope | Not applicable; leaves are opaque to the contract |

## Revocation (#661)

- `revoke_batch(caller, attester, root)` revokes the whole batch.
- `revoke_batch_leaf(caller, attester, root, leaf_index)` revokes one leaf.
- Either may be called by the anchoring attester or a holder of the Revoker
  role, matching single-attestation revocation and withdrawal.
- Per-leaf state is a 128-bit bitmap per 128-leaf segment
  (`BatchLeafRevoked(attester, root, segment)`), so storage grows with
  revoked segments, not with leaves.
- Verifiers call `is_batch_leaf_revoked(attester, root, leaf_index)` after a
  successful inclusion proof and return `revoked` when it is true.
- Events: `AttestationBatchRevoked`, `AttestationBatchLeafRevoked`.
