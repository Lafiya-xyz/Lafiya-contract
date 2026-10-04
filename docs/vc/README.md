# Lafiya Verifiable Credentials

This directory contains the W3C Verifiable Credentials 2.0 artefacts for
Lafiya's `LafiyaEmergencyRecordAttestation` credential type. See
[ADR-0012](../adr/0012-w3c-vc-export-format.md) for the decision record.

## Files

| File | Description |
|---|---|
| `context.jsonld` | The JSON-LD context published at `https://lafiya.xyz/ns/credentials/v1` |
| `example-credential.json` | An example signed `LafiyaEmergencyRecordAttestation` VC |

## What is a Lafiya VC?

A `LafiyaEmergencyRecordAttestation` is a W3C VC 2.0 that represents a
community health worker's on-chain attestation that they verified a patient's
emergency health record. It contains:

- **`issuer`** — the attester's DID (see [ADR-0013](../adr/0013-did-resolution-for-attesters.md))
- **`credentialSubject`** — the patient's `recordCommitment` (LRC-1 hash) and
  `commitmentVersion`. **No health data is included.**
- **`credentialStatus`** — a `StatusList2021Entry` pointing at a status-list
  derived from on-chain revocation state (queried via
  `attestation-registry::get_attestation`)
- **`evidence`** — the Soroban contract address, ledger, and transaction hash
  anchoring this credential to the on-chain record
- **`proof`** — a `DataIntegrityProof` with `eddsa-jcs-2022`, signed by the
  attester's Ed25519 key

The credential carries **no personal health data**. The `recordCommitment` is an
opaque LRC-1 hash; reconstructing the underlying record requires the patient's
consent and their off-chain data from `lafiya-web`. This is consistent with
[ADR-0001](../adr/0001-hash-only-on-chain-footprint.md).

## Validating the example credential

Once the `lafiya-vc` crate is implemented (post-M1), you can validate the example
with the `@digitalbazaar/vc` reference implementation:

```bash
npm install -g @digitalbazaar/vc
npx vc verify --credential docs/vc/example-credential.json
```

Or via the CLI (post-M1):

```bash
lafiya-cli attestation verify-vc \
    --credential docs/vc/example-credential.json \
    --network testnet
```

## Producing a credential (post-M1)

```bash
lafiya-cli attestation export-vc \
    --record-hash a3f4e2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3 \
    --network testnet \
    --attester-key-file ~/.lafiya/attester-key.json \
    --out my-attestation.json
```

The command fetches the on-chain attestation for the given `record-hash`,
resolves the attester's DID document, constructs the VC, signs it with
`eddsa-jcs-2022`, and writes the result to `--out`.

## JSON-LD context

The context at `context.jsonld` defines:

- `LafiyaEmergencyRecordAttestation` — the credential type
- `recordCommitment` — hex-encoded LRC-1 commitment (32 bytes)
- `commitmentVersion` — version string (currently always `lrc1`)
- `SorobanAttestationRecord` — evidence type for on-chain anchoring
- `network`, `contract`, `ledger`, `txHash` — evidence fields

The context must be published at `https://lafiya.xyz/ns/credentials/v1` before
any credential using it is presented to an external verifier. For pre-alpha
testing, a local copy can be used via the `documentLoader` option in the
`@digitalbazaar/vc` API.

## Privacy

VCs in this profile contain:
- The attester's DID (public, on-chain identity)
- A `recordCommitment` (opaque hash — not reversible without the patient's consent
  and access to `lafiya-web`)
- On-chain evidence (contract address, ledger, tx hash — all public)

They do **not** contain: patient name, date of birth, blood group, genotype,
allergies, medications, or any other health data.
