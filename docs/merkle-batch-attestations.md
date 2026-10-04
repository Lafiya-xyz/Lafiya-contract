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
on-chain lookup, history, and revocation are required. A batch root is one
indivisible commitment: the contract cannot selectively revoke one leaf.
