# System Overview for Auditors

## What Lafiya does

Lafiya lets an allowlisted **attester** (a clinic, lab, or health
authority) publish, on Stellar, that it vouches for a health record. Only a
32-byte **commitment** of the record is stored on-chain
([ADR 0001](../docs/adr/0001-hash-only-on-chain-footprint.md)); the record
itself stays off-chain. A **verifier** later recomputes the commitment
from the record it holds (LRC-1, [ADR 0008](../docs/adr/0008-record-commitment-canonicalization.md))
and checks that an allowlisted attester attested it and that the
attestation was not revoked.

## Contracts

```
                 ┌───────────────────────┐
                 │   multisig-account    │  N-of-M ed25519 custom account
                 │  (__check_auth)       │  (admin of both registries)
                 └──────────┬────────────┘
                 admin auth │ admin auth
          ┌─────────────────┴───────────────────┐
          ▼                                     ▼
┌──────────────────────┐   is_attester()  ┌─────────────────────────┐
│  attester-registry   │◄─────────────────│  attestation-registry   │
│  allowlist, metadata,│  cross-contract  │  attest / revoke /      │
│  suspension, cap     │      call        │  history per record_hash│
└──────────────────────┘                  └─────────────────────────┘
          ▲                                     ▲
          │ admin ops                           │ attest (attester auth)
       operators                             attesters
```

- **attester-registry**: the allowlist. The admin adds, removes, suspends,
  and reinstates attesters, sets metadata and an allowlist cap, pauses the
  contract, and upgrades or migrates it. Anyone can query `is_attester`.
- **attestation-registry**: stores attestations keyed by `record_hash`,
  with a bounded history (`MAX_HISTORY = 10`). `attest` requires the
  attester's own auth and a successful `is_attester` call on the
  configured registry. The admin can revoke attestations, repoint the
  registry (`set_attester_registry`), pause, and upgrade.
- **multisig-account**: a Soroban custom account. `__constructor` stores
  the signer set and threshold. `__check_auth` accepts a set of signatures
  sorted by public key, rejects duplicates, unknown signers, fewer than
  the threshold, and more than the signer count.

## Trust assumptions

1. The **admin** of each registry is fully trusted: it can upgrade the
   Wasm to arbitrary code. In production, the admin is the multisig
   account, so trust moves to "at least a threshold of signers is honest".
2. **Attesters** are trusted only for their own attestations. A malicious
   attester must not be able to affect another attester's attestations or
   the allowlist.
3. **Anyone** can call the view functions and submit transactions. No
   state-changing function may be callable without the auth described in
   [invariants.md](invariants.md).
4. The **Soroban host** correctly enforces `require_auth`, storage
   isolation, and TTL semantics.
5. **Off-chain verifiers** implement LRC-1 correctly and check revocation
   and the attester's current status, as described in
   [ADR 0006](../docs/adr/0006-attestation-revocation-semantics.md).

## Admin topology

```
multisig-account (threshold t of n signers)
  ├── admin of attester-registry
  └── admin of attestation-registry
```

Admin transfer is two-step on both registries (`propose_admin`, then
`accept_admin` by the proposed address). The multisig signer set is fixed
at construction; changing it means deploying a new multisig and
transferring admin to it.

## Accepted risks

| Risk | Decision |
|---|---|
| The multisig authorizes **any** invocation that has enough valid signatures; `auth_contexts` are ignored | Accepted for pre-alpha — [ADR 0007](../docs/adr/0007-unscoped-multisig-authorization.md) |
| A removed or suspended attester's past attestations remain on-chain; verifiers must check current attester status and revocation | Accepted — [ADR 0006](../docs/adr/0006-attestation-revocation-semantics.md) |
| A single admin (before multisig handover) can upgrade to arbitrary Wasm | Accepted — [ADR 0003](../docs/adr/0003-single-admin-initial-model.md) |

## Further reading

- Storage layout and migrations: [docs/architecture/storage-versioning.md](../docs/architecture/storage-versioning.md)
- Events: [docs/events.md](../docs/events.md), [docs/architecture/event-indexing.md](../docs/architecture/event-indexing.md)
- Error codes: [docs/error-codes.md](../docs/error-codes.md)
- Upgrade procedure: [docs/runbooks/contract-upgrade.md](../docs/runbooks/contract-upgrade.md)
- Release manifest: [ADR 0010](../docs/adr/0010-release-manifest-and-compatibility.md)
