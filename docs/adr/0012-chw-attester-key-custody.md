# ADR-0012: CHW attester key custody and account type

## Status

Proposed

## Context

Every attestation is only as trustworthy as the Community Health Worker's
(CHW's) signing key. The contracts treat an attester `Address` as an
unforgeable identity (`attest` calls `attester.require_auth()`), but nothing
specified how that key is generated, stored, backed up, rotated, or revoked in
the field. Field conditions are harsh: shared or low-end Android phones, lost
or stolen devices, intermittent connectivity, limited technical training, and
sometimes coercion.

The operational policy that implements this decision is
[docs/security/chw-key-custody.md](../security/chw-key-custody.md).

## Options considered

| Criterion | A. Classic `G...` ed25519 key in device keystore | B. Passkey smart account (secp256r1 / WebAuthn contract account) | C. Custodial key held by the programme |
| --- | --- | --- | --- |
| Key extractability | Android Keystore does not support ed25519 in hardware on most devices, so the seed usually lives in app storage, encrypted by a keystore-wrapped AES key. Extractable from a rooted device. | Private key is generated and held in StrongBox/TEE or a synced passkey provider; never leaves secure hardware. | Key sits on programme servers; any server compromise signs as every CHW. |
| Seed phrase | Yes, 24 words to protect or lose. | None. | None for the CHW. |
| Non-repudiation (the attestation proves *this CHW* verified) | Good if the key never left the device. | Strongest: user-presence (biometric/PIN) gate on each signature. | **Lost.** The programme can sign on behalf of any CHW; attestations prove only that the programme's server signed. |
| Sponsored/relayed transactions | Works (auth entry signed by the CHW; relayer pays fees). | Works: `__check_auth` on the smart account; relayer pays fees and the CHW needs no XLM. | Works. |
| Cost | Base account reserve (1 XLM) per CHW, sponsorable. | One contract deployment per CHW plus higher per-signature verification cost (secp256r1 is costlier than ed25519). | Lowest. |
| Recovery after device loss | Seed phrase restore (poor field usability) or rotation to a new key. | Rotate the signer on the smart account, or register a new account and rotate the attester address. | Trivial. |
| Usability | Poor: seed backup training burden. | Best: biometric prompt, no secrets to write down. | Best. |

## Decision

Adopt **Option B, a passkey-based smart account**, as the target CHW attester
account type, with **Option A (ed25519 key wrapped by the Android hardware
keystore)** as the interim account type until the contract dependencies below
ship. **Option C is rejected** for attester keys. It removes non-repudiation,
which is the property attestations exist to provide. A programme that
nonetheless operates custodially must disclose it and must not show those
attestations to responders as CHW-verified.

Rationale: B gives the strongest security (hardware-bound, user-presence per
signature, no seed phrase) and the best field usability, and it composes with
sponsored transactions so CHWs never hold XLM. Its higher per-signature cost is
acceptable at attestation volumes, and relayers absorb it.

## Contract-feature dependencies

| Need | Contract feature | Status | Issue | Effort estimate |
| --- | --- | --- | --- | --- |
| Passkey signers | secp256r1 / WebAuthn signer support in the account contract | Missing | [#356](https://github.com/Lafiya-xyz/Lafiya-contract/issues/356) | L (2–3 weeks incl. audit surface) |
| Signer rotation on the CHW smart account | signer-set rotation via self-authorization | Missing | [#355](https://github.com/Lafiya-xyz/Lafiya-contract/issues/355) | M (1 week) |
| CHW never holds XLM | sponsored, relayer-submitted attestations | Missing | [#347](https://github.com/Lafiya-xyz/Lafiya-contract/issues/347) | M (1–2 weeks) |
| Lost/stolen device, keep identity | attester key rotation preserving identity and metadata | Missing | [#331](https://github.com/Lafiya-xyz/Lafiya-contract/issues/331) | M (1 week) |
| Theft discovered late | compromise window `mark_attester_compromised(attester, since)` | Missing | [#337](https://github.com/Lafiya-xyz/Lafiya-contract/issues/337) | M (1 week) |
| Offboarding | `renounce_attester` | Missing | [#332](https://github.com/Lafiya-xyz/Lafiya-contract/issues/332) | S (2–3 days) |
| Re-enrollment after suspension | orphaned suspended key must not make a new attester "born suspended" | Bug | [#324](https://github.com/Lafiya-xyz/Lafiya-contract/issues/324) | S (1–2 days) |
| Loss/theft suspension | `suspend_attester` / `reinstate_attester` (admin) | **Exists** | n/a | n/a |
| Duress | attester-initiated quiet self-suspension (`self_suspend`, auth by the attester, reinstatement admin-only) | Missing | Not yet filed (tracked as follow-up) | S (2–3 days) |

## Consequences

### Positive

- Attestations remain non-repudiable and hardware-bound.
- No seed phrases in the field; recovery is a rotation, not a restore.
- Clear, issue-linked path from interim (A) to target (B).

### Negative / trade-offs

- Option B depends on four unshipped contract features; until then, Option A's
  weaker extractability guarantees apply.
- Passkey support varies across low-end Android devices; the device policy sets
  a floor that some existing handsets will not meet.
- Synced passkeys (cloud-backed) weaken hardware binding. The policy requires
  device-bound credentials for attester keys.

## Follow-up

- File the `self_suspend` duress issue and link it from the table above.
- Revisit this ADR once #356, #355, #347, and #331 ship, and move the status to
  Accepted.
