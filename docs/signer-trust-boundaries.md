# Signer Trust Boundaries

**Related issue:** #399 — Hardware wallet (Ledger) and remote HSM/KMS signing

**Implementation:** `crates/lafiya-cli/src/signer.rs`

---

## Overview

The `Signer` trait makes the signing back-end pluggable. Select a back-end via
`--signer <uri>`:

| URI | Back-end | Key location |
|---|---|---|
| `identity://<name>` (default) | stellar CLI file identity | Local disk |
| `ledger://<index>` | Ledger hardware wallet | Device secure element (never exported) |
| `gcpkms://<resource-path>` | Google Cloud KMS (ed25519) | GCP HSM (never exported) |
| `awskms://<key-id>` | AWS KMS (ed25519) | AWS CloudHSM (never exported) |

---

## Trust boundaries per back-end

### `identity://` — file key (default)

| Property | Detail |
|---|---|
| Key leaves device? | Yes — stored as a file on disk |
| Blind-hash risk | None — operator controls the environment |
| Auditability | Local; no external audit log |
| When to use | Development, non-mainnet, single-operator setups |
| Risk | Disk compromise or memory dump can expose the key. Never use for mainnet admin keys. |

### `ledger://` — Ledger hardware wallet

| Property | Detail |
|---|---|
| Key leaves device? | Never — ed25519 key stays inside the secure element |
| Blind-hash risk | **Yes for Soroban auth entries.** The Stellar app cannot display the decoded invocation tree when signing a `HashIdPreimage::SorobanAuthorization` hash. The CLI always prints the full tree on the host terminal before sending the APDU. |
| Auditability | Device screen confirmation; no remote log |
| When to use | Mainnet admin keys, high-value operations |
| APDU commands | Envelope: `0xE0 0x02`. Hash signing (Soroban auth): `0xE0 0x04` (requires "Allow blind signing" in Stellar app settings) |
| Stellar app version | ≥ 7.x recommended |

**Operator procedure for Soroban auth entry signing (ledger):**

1. The CLI prints the decoded invocation tree (contract, function, arguments, sub-invocations).
2. Verify the output matches your intent before touching the device.
3. The CLI then shows:
   ```
   ⚠️  LEDGER BLIND HASH SIGNING
   The Ledger Stellar app cannot display the Soroban authorization
   invocation tree. The host has already printed it above.
   You are about to sign hash: <hex>
   Description: <operation>
   Proceed? [y/N]
   ```
4. Confirm on the device only after verifying the host terminal output.

### `gcpkms://` — Google Cloud KMS

| Property | Detail |
|---|---|
| Key leaves device? | Never — GCP HSM boundary |
| Blind-hash risk | Yes — KMS signs opaque bytes; CLI displays operation before calling |
| Auditability | Cloud Audit Logs: `cloudkms.cryptoKeyVersions.useToSign` |
| Ed25519 support | GA since early 2024. Purpose: `ASYMMETRIC_SIGN`, algorithm: `EC_SIGN_ED25519` |
| API | `POST .../cryptoKeyVersions/{version}:asymmetricSign` |
| Credentials | `GOOGLE_APPLICATION_CREDENTIALS` or Application Default Credentials |
| IAM | Requires `roles/cloudkms.signerVerifier` or `cloudkms.cryptoKeyVersions.useToSign` |
| Risk | IAM misconfiguration can grant signing to unintended principals. Audit grants regularly. |

### `awskms://` — AWS KMS

| Property | Detail |
|---|---|
| Key leaves device? | Never — AWS CloudHSM boundary; keys marked non-exportable |
| Blind-hash risk | Yes — same as GCP KMS |
| Auditability | AWS CloudTrail: `kms:Sign` events |
| Ed25519 support | Available since 2023. Key spec: `KEY_SPEC=ED25519`, signing algorithm: `ED25519`, MessageType: `RAW` |
| API | `kms:Sign` via AWS SDK |
| Credentials | Standard AWS credential chain (`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / instance profile) |
| IAM | Requires `kms:Sign` on the key ARN |
| Risk | Multi-region key replication; scope key policy to minimum required principals |

---

## All signing paths use `Signer`

1. **Transaction envelope** (`is_soroban_auth_entry = false`) — full XDR envelope hash. Hardware wallets can display this.
2. **Soroban authorization entry** (`is_soroban_auth_entry = true`) — `HashIdPreimage::SorobanAuthorization` hash. All back-ends sign a hash only; the CLI always prints the decoded invocation tree first.
3. **Multisig ceremony** — each signer calls `Signer::sign` independently; the client assembles the quorum before submission.

---

## Recommendations by environment

| Environment | Recommended back-end |
|---|---|
| Local development | `identity://` |
| Testnet — single operator | `identity://` |
| Testnet — team / CI | `gcpkms://` or `awskms://` |
| Mainnet admin (human) | `ledger://` |
| Mainnet admin (automated) | `gcpkms://` or `awskms://` with audit log review |

Never use `identity://` for mainnet admin keys.

---

## References

- `crates/lafiya-cli/src/signer.rs` — implementation and unit tests
- [SECURITY.md](../../SECURITY.md)
- [ADR-0007: Unscoped Multisig Authorization](adr/0007-unscoped-multisig-authorization.md)
- Stellar Ledger app: https://github.com/LedgerHQ/app-stellar
- GCP KMS Ed25519: https://cloud.google.com/kms/docs/algorithms
- AWS KMS Ed25519: https://docs.aws.amazon.com/kms/latest/developerguide/asymmetric-key-specs.html
