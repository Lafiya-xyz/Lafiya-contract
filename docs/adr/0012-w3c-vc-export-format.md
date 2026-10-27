<!-- adr-redirect: 0022-w3c-vc-export-format.md -->
# Moved: ADR-0022

| Field | Value |
|---|---|
| **Status** | Proposed |
| **Date** | 2026-09-26 |
| **Deciders** | Lafiya core team |
| **Relates to** | ADR-0001 (hash-only on-chain footprint), ADR-0008 (LRC-1 commitment) |

## Context

The README states that Lafiya's attestation layer is *informed by the W3C Verifiable
Credentials data model*, but the project currently produces no VC artefacts.
National health ID systems (Nigeria NHIA), digital public goods registries, and
partner NGOs increasingly require VCs as the interoperability format for verified
health records. Without a standards-compliant VC profile, a Lafiya attestation can
only be consumed by software that understands Lafiya's proprietary contract schema.

A VC profile would let:
- Responders' verifier apps accept Lafiya attestations alongside any other W3C-
  compatible credential without Lafiya-specific integration code.
- CHWs carry a portable credential in a standards-compatible wallet.
- Grant auditors verify attestation counts against an independent W3C validator
  rather than a custom RPC query.

## Decision

We adopt **W3C Verifiable Credentials Data Model 2.0** (VC 2.0) as the export
format, with the following specifics:

### Proof format

**`DataIntegrityProof` with `eddsa-jcs-2022`** (JSON Canonicalization Scheme +
Ed25519, from the W3C VC Data Integrity specification suite).

Rationale over the alternatives:
- `eddsa-rdfc-2022` (RDF Dataset Normalisation) requires a full JSON-LD
  processor and RDF graph normalisation — significant runtime dependency for an
  edge-device verifier.
- JWT-VC (`vc+sd-jwt`) hides the JSON-LD structure and makes the proof opaque to
  non-JWT tooling; it adds JOSE as a required dependency.
- `eddsa-jcs-2022` (JCS) signs over a deterministically serialised JSON object,
  implemented by any language with a JCS library (available in Rust, TypeScript,
  and Python). This keeps the verifier simple and the artefact human-readable.

The attester's Ed25519 signing key is identified via their DID — see ADR-0013.

### Credential type

`LafiyaEmergencyRecordAttestation`

```
@context:
  - https://www.w3.org/ns/credentials/v2
  - https://lafiya.xyz/ns/credentials/v1   (JSON-LD context, see docs/vc/context.jsonld)

type: [VerifiableCredential, LafiyaEmergencyRecordAttestation]
```

### `credentialSubject`

Only the record commitment and its schema version are included — no health data:

```json
{
  "id": "did:pkh:stellar:<patient-account>",
  "recordCommitment": "<hex-encoded 32-byte LRC-1 hash>",
  "commitmentVersion": "lrc1"
}
```

### `credentialStatus`

A `BitstringStatusListEntry` (W3C Bitstring Status List, successor to
`StatusList2021Entry`) pointing at a status-list endpoint that is derived from
on-chain revocation state (checked via `attestation-registry::get_attestation`).
Pre-alpha: the status-list is generated on demand by the `lafiya-cli attestation
export-vc` command from the current chain state; a persistent status-list endpoint
is deferred to M2. The status-list contract, index allocation scheme, and
verifier sync protocol are specified in ADR-0014; this section only defines how
exported credentials bind to it.

#### Index allocation and binding

Each exported credential carries a status-list entry of the form:

```json
{
  "id": "<status-list URL>#<index>",
  "type": "BitstringStatusListEntry",
  "statusPurpose": "revocation",
  "statusListIndex": "<decimal index as string>",
  "statusListCredential": "<status-list URL>"
}
```

- `statusListIndex` is the position of the record's bit in the status list
  bitstring. It is **not** derived from the record hash or any on-chain
  identifier: the index is assigned by the status-list contract from a
  monotonically increasing counter and stored in a per-record slot that is
  itself keyed by a salted commitment (see ADR-0014, "Index correlation").
  The export command reads the assigned index from the contract; it never
  computes it locally.
- `statusPurpose` is `revocation` for emergency-record attestations. A record
  that is revoked on-chain has its bit set to `1`; `0` means "not revoked".
  (Bitstring Status List also defines a `suspension` purpose; Lafiya does not
  use it — revocation is final.)
- `statusListCredential` identifies the status-list VC whose `credentialSubject`
  contains the `encodedList` (a GZIP-compressed, base64url-encoded bitstring)
  and whose `proof` is the signed root. The URL is versioned
  (`.../status-list/<version>`); a new version is published whenever the
  bitstring changes, and the credential's `validFrom`/`validUntil` bound the
  window in which that version is authoritative.

#### Verifier validation against the signed root

A verifier that holds an exported credential validates status as follows:

1. Resolve `statusListCredential` and verify its `DataIntegrityProof`
   (`eddsa-jcs-2022`) against the status-list issuer's DID — the same proof
   suite and DID resolution rules used for the attestation VC itself. Reject
   the list if the proof is invalid or the issuer is not trusted.
2. Check that the status-list credential's `credentialSubject.type` is
   `BitstringStatusList` and that its `statusPurpose` matches the entry's
   `statusPurpose`.
3. Decode `encodedList` (base64url → GZIP → bitstring) and read the bit at
   `statusListIndex`. Out-of-range indices are treated as an error, not as
   "not revoked".
4. If the bit is `1`, the credential is revoked. If `0`, the credential is
   valid *as of that list version*; the verifier records the list version and
   `validUntil` so it can detect staleness on its next sync.

Because the bitstring is signed as a whole, a verifier can cache one status-list
credential and answer revocation checks for every credential that references it
without per-record RPC. The verifier SDK (`packages/verifier-sdk`) exposes
`syncStatusList(url)` and `checkStatus(credential)` implementing steps 1–4;
the sync demo and measured size/sync-time figures for 1M entries are recorded
in ADR-0014.

### `evidence`

```json
[{
  "type": "SorobanAttestationRecord",
  "network": "<network passphrase>",
  "contract": "<attestation-registry C... address>",
  "ledger": <ledger sequence>,
  "txHash": "<transaction hash>"
}]
```

### Export command

```
lafiya-cli attestation export-vc \
    --record-hash <hex> \
    --network testnet \
    --attester-key-file <path-to-ed25519-signing-key.json> \
    --out credential.json
```

The command:
1. Fetches the on-chain attestation for `record_hash`.
2. Resolves the attester's DID document (per ADR-0013) to get the verification
   method.
3. Constructs the VC JSON.
4. Signs with `eddsa-jcs-2022` using the provided key material.
5. Writes the signed VC to `--out`.

### Verify command

```
lafiya-cli attestation verify-vc --credential credential.json --network testnet
```

Checks:
1. Proof signature is valid (`eddsa-jcs-2022`).
2. The attester DID resolves to a key matching the proof's `verificationMethod`.
3. The on-chain attestation for `credentialSubject.recordCommitment` exists and
   is not revoked.

### Off-chain validator

The example VC in `docs/vc/example-credential.json` must pass the
[`@digitalbazaar/vc`](https://github.com/digitalbazaar/vc) npm validator. The
check is documented in `docs/vc/README.md` and will be added to CI as a non-
blocking step once the `vc` command is implemented (post-M1).

## Alternatives considered

### JWT-VC (`vc+sd-jwt`)

Rejected: adds JOSE as a required dependency; the proof is opaque to non-JWT
tooling; selective disclosure is not needed here (the VC already contains no
health data).

### `eddsa-rdfc-2022`

Rejected: requires a full JSON-LD processor and RDF normalisation (N-Quads) at
runtime — heavy dependency for edge-device verifiers and CLI tools.

### Custom proof format

Rejected: defeats the interoperability purpose entirely.

## Consequences

- **Adds** `docs/vc/` directory: JSON-LD context, example credential, README.
- **Adds** `crates/lafiya-vc/` crate (post-M1): VC construction and
  `eddsa-jcs-2022` signing/verification using the `ed25519-dalek` crate already
  available transitively in the workspace.
- **Adds** `attestation export-vc` and `attestation verify-vc` subcommands to
  `lafiya-cli` (post-M1).
- **Does not** store any VC artefact on-chain — VC signing happens off-chain;
  the on-chain state is only the source of truth for `evidence` and status.
- **Privacy:** the VC contains `recordCommitment` (a hash) and the attester's
  DID; no patient identity or health data. This is consistent with ADR-0001.
- Requires the attester to hold an Ed25519 signing key whose public key is
  registered in their DID document (see ADR-0013 for DID method).
