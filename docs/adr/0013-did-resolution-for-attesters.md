# ADR-0013: DID resolution method for attester identity

| Field | Value |
|---|---|
| **Status** | Proposed |
| **Date** | 2026-09-26 |
| **Deciders** | Lafiya core team |
| **Relates to** | ADR-0012 (W3C VC export), ADR-0001 (hash-only on-chain footprint) |

## Context

An attester is currently identified on-chain only by their Stellar account
address (`G...`). A verifier outside the Lafiya app cannot:

1. Discover the attester's current public key without calling Lafiya-specific
   contract functions and knowing Lafiya's conventions.
2. Find which health facility or license authority backs the attester.
3. Detect that an attester has been suspended or removed from the allowlist.

For W3C VC interoperability (ADR-0012) the credential's `issuer` must be a DID
whose document can be resolved by any standards-compliant verifier to find the
attester's `verificationMethod` (public key). The DID must also reflect the
attester's current operational status — suspended or removed attesters must not
appear as active issuers.

### Candidate methods

| Method | Infrastructure needed | Key rotation | Metadata | Privacy |
|---|---|---|---|---|
| `did:pkh:stellar` | None (address = DID) | None — key is the address | None | Good |
| `did:web:lafiya.xyz:attesters:<addr>` | HTTPS server under our control | Via server update | Full | OK |
| `did:lafiya:<addr>` (custom) | Resolver using contract state | Mapped to `update_attester_info` | On-chain fields | Good |

## Decision

We adopt **`did:pkh:stellar:<stellar-address>`** as the attester DID method for
the initial implementation, with a documented upgrade path to `did:web` once the
lafiya.xyz domain and metadata hosting are production-ready.

Concretely, for an attester with Stellar address `GAHJJ...`:

```
DID: did:pkh:eip155:stellar:GAHJJ...
```

(The `eip155` namespace is the existing PKH chain namespace convention for
non-EVM chains; a Stellar-specific CAIP-2 chain identifier should be used when
standardised. For pre-alpha, `eip155:stellar` is an accepted placeholder.)

### Rationale

- **Zero infrastructure.** No DNS, no server, no database migration. The DID
  Document can be reconstructed deterministically from the Stellar address and
  the attester's Ed25519 signing key (which they supply when constructing a VC).
- **Privacy.** No metadata beyond what is already on-chain is published. A CHW's
  license document hash and region are stored in `attester_info` on-chain; the DID
  document does not need to repeat them.
- **Good enough for M1.** The VC proof only needs a `verificationMethod` — a
  public key tied to the attester's identity. `did:pkh` provides exactly that.

### Known limitation and upgrade path

`did:pkh` does not support key rotation or service endpoints. If an attester's
signing key is compromised they must be removed and re-added with a new address.
This is acceptable at pre-alpha scale (small allowlist, admin-gated membership).

When the project matures to mainnet, we will evaluate:
1. **`did:web:lafiya.xyz:attesters:<addr>`** — adds service endpoints and
   supports metadata for license authority and region in a human-readable URL;
   trust anchored in `lafiya.xyz` DNS + TLS.
2. **`did:lafiya:<addr>`** (custom method) — fully on-chain, resolver reads from
   `attester-registry`; more decentralised but requires a published method spec
   and a Universal Resolver driver.

The upgrade is non-breaking for VC consumers: the `issuer.id` field changes but
the `credentialSubject.recordCommitment` and `evidence` fields are stable.

## Method spec (pre-alpha `did:pkh:stellar`)

### Create

A `did:pkh:stellar:<address>` DID is created implicitly when an attester is
added to the `attester-registry` via `add_attester` or `add_attester_with_info`.
No separate DID registration step is required.

### Resolve

Given `did:pkh:eip155:stellar:<stellar-address>`, the resolver:

1. Calls `attester-registry::is_attester(<stellar-address>)` on the configured
   network. If `false`, the DID Document's `active` flag is `false`.
2. Calls `attester-registry::get_attester_status(<stellar-address>)` to
   determine suspension state.
3. Constructs the DID Document:

```json
{
  "@context": [
    "https://www.w3.org/ns/did/v1",
    "https://w3id.org/security/suites/ed25519-2020/v1"
  ],
  "id": "did:pkh:eip155:stellar:<stellar-address>",
  "verificationMethod": [{
    "id": "did:pkh:eip155:stellar:<stellar-address>#key-1",
    "type": "Ed25519VerificationKey2020",
    "controller": "did:pkh:eip155:stellar:<stellar-address>",
    "publicKeyMultibase": "<base58btc-encoded Ed25519 public key>"
  }],
  "assertionMethod": [
    "did:pkh:eip155:stellar:<stellar-address>#key-1"
  ],
  "deactivated": <true if is_attester == false or suspended == true>
}
```

The `publicKeyMultibase` is derived from the Ed25519 public key embedded in the
Stellar `G...` address (the last 32 bytes of the decoded strkey).

### Update

There is no explicit update operation at `did:pkh`. Attester metadata updates
(`update_attester_info`) change the `attester_info` stored on-chain but do not
change the DID or its `verificationMethod`.

Key rotation requires the admin to call `remove_attester` on the old address and
`add_attester` on the new address.

### Deactivate

A DID is considered deactivated when:
- `is_attester(<address>)` returns `false` (never added, or removed), or
- `get_attester_status(<address>).suspended == true`.

The resolver sets `"deactivated": true` in the DID Document metadata in either
case.

## DID resolver implementation

### Rust library (`crates/lafiya-did` — post-M1)

A `LafiyaDidResolver` that implements the
[`did-resolver`](https://docs.rs/did-resolver) trait, resolving
`did:pkh:eip155:stellar:*` DIDs by querying the `attester-registry` contract via
the Stellar RPC.

```
lafiya-cli attestation verify-vc     # uses LafiyaDidResolver internally
```

### Universal Resolver driver (post-M1)

An HTTP endpoint conforming to the
[Universal Resolver driver spec](https://github.com/decentralized-identity/universal-resolver)
that wraps `LafiyaDidResolver`. This makes Lafiya DIDs resolvable via the DIF
Universal Resolver at `https://dev.uniresolver.io/` without any Lafiya-specific
code in the verifier.

## Resolver test matrix

Tests against a local quickstart or mock contract cover:

| State | `is_attester` | `suspended` | Expected `deactivated` |
|---|---|---|---|
| Active attester | `true` | `false` | `false` |
| Suspended attester | `true` | `true` | `true` |
| Removed attester | `false` | — | `true` |
| Never added | `false` | — | `true` |

## Privacy review

The DID Document derived from `did:pkh:stellar` contains:
- The Stellar address (public, already on-chain in the attester allowlist).
- The Ed25519 public key (derived from the address — also public).
- The `deactivated` flag (derived from on-chain state — also public).

It does **not** contain: the attester's real name, employer, license number, or
region. The `license_hash` and `region` stored in `attester_info` are **not
included** in the DID Document, consistent with ADR-0001's principle of minimal
on-chain disclosure.

## Consequences

- **Adds** `docs/specs/did-method-lafiya-pkh.md` — the method spec document (see
  `docs/specs/`).
- **Adds** `crates/lafiya-did/` (post-M1) — Rust resolver library.
- **Adds** a Universal Resolver driver (post-M1, separate repo).
- **No contract changes** — resolver reads existing `attester-registry` state.
- **No new on-chain data** — the DID is derived from data already on-chain.
- The `did:pkh` approach binds key identity to the Stellar address; if the
  attester's Stellar private key is compromised, both the address and the DID are
  compromised. This risk is documented in `docs/specs/did-method-lafiya-pkh.md`
  and accepted at pre-alpha scale.
