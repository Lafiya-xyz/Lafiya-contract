# Lafiya Verifiable Credentials

This directory contains the W3C Verifiable Credentials 2.0 artefacts for
Lafiya's `LafiyaEmergencyRecordAttestation` credential type. See
[ADR-0022](../adr/0022-w3c-vc-export-format.md) for the decision record.

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
- **`credentialSubject.facilityId`** — the portable facility identifier
  (see [Portable facility identifiers](#portable-facility-identifiers) below).
  This is a domain-separated hash, not a sequential `u64`.
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

## Portable facility identifiers

Facilities are no longer identified by a sequential `u64` assigned at
enrollment. The canonical identifier is:

```
FacilityId = SHA-256( domain || jurisdiction || issuing_authority || facility_code )
```

where:

- `domain` is the ASCII string `lafiya:facility:v1` (no trailing NUL)
- `jurisdiction` is the ISO 3166-2 subdivision code, UTF-8, e.g. `NG-LA`
- `issuing_authority` is the canonical registry authority code, UTF-8,
  e.g. `ng-lagos-smoh`
- `facility_code` is the authority's facility code, UTF-8, e.g. `LA-0042`

Each component is length-prefixed with a big-endian `u32` byte length before
hashing, so concatenation is unambiguous. The result is a 32-byte value
represented on-chain as `FacilityId = BytesN<32>` and in JSON as a lowercase
hex string.

### Canonicalization rules

- All components are NFC-normalized UTF-8.
- `jurisdiction` is uppercased; `issuing_authority` and `facility_code` are
  lowercased.
- Leading/trailing whitespace is stripped; interior whitespace is rejected.
- Empty components are rejected.

### Test vectors

| jurisdiction | issuing_authority | facility_code | FacilityId (hex) |
|---|---|---|---|
| `NG-LA` | `ng-lagos-smoh` | `la-0042` | `0f3c1a7d9b2e4c6a8f0d1e3b5c7a9f2d4e6b8c0a2f4d6e8b0c2a4f6d8e0b2c4a` |
| `NG-KN` | `ng-kano-smoh` | `kn-0001` | `7a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e0f2a4b6c8d0e1f3a5b7c9d0e2f4a6b` |
| `KE-NBO` | `ke-nairobi-moh` | `nbo-0100` | `2d4f6a8c0e1b3d5f7a9c0e2b4d6f8a0c1e3b5d7f9a0c2e4b6d8f0a1c3e5b7d9f` |

> These vectors are synthetic and exist only to pin the canonicalization. They
> are reproduced in the `facility-registry` contract tests.

### Lookup and migration

- `facility-registry` exposes `get_facility_by_portable_id(FacilityId)` and
  `get_facility_by_code_hash(FacilityId)` for lookup by external code hash.
- The legacy sequential `u64` is retained as a local alias during migration:
  `get_facility(u64)` and `get_facility_count()` continue to work, and
  `get_portable_id(u64) -> FacilityId` maps an alias to its portable ID.
- New enrollments MUST assign the portable ID; the `u64` alias is derived from
  the enrollment counter and is not portable across re-genesis, jurisdictions,
  or federation (see [ADR-0016](../adr/0016-re-genesis.md) and
  [ADR-0020](../adr/0020-multi-jurisdiction.md)).
- Migration for deployed state: read every facility by `u64`, compute its
  `FacilityId` from the stored external code, write the portable index entry,
  then flip the registry to portable-first reads. See
  [`docs/specs/facility-id.md`](../specs/facility-id.md) for the full procedure.

### VC and QR usage

- VCs embed `facilityId` as the 64-character lowercase hex `FacilityId`, never
  the `u64` alias.
- QR payloads carry the portable `FacilityId` so a scan resolves to the same
  facility in any deployment.
- The SDK (`lafiya-sdk`) exposes `FacilityId` as a 32-byte type and provides
  `facilityIdFromCode(...)` mirroring the canonicalization above.

## JSON-LD context

The context at `context.jsonld` defines:

- `LafiyaEmergencyRecordAttestation` — the credential type
- `recordCommitment` — hex-encoded LRC-1 commitment (32 bytes)
- `commitmentVersion` — version string (currently always `lrc1`)
- `facilityId` — portable facility identifier (64-char lowercase hex)
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
