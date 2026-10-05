# CHW Attester Key Custody and Device Policy

Decision record: [ADR-0012](../adr/0012-chw-attester-key-custody.md).
Applies to every Community Health Worker (CHW) whose address is allowlisted in
`attester-registry`.

## 1. Account type

- **Target:** a passkey-based smart account (secp256r1 / WebAuthn), with a
  **device-bound** credential in StrongBox or TEE. Synced (cloud-backed)
  passkeys are not permitted for attester keys.
- **Interim:** a classic `G...` ed25519 account. The seed is generated
  on-device, encrypted at rest with an AES-256-GCM key held in the Android
  hardware keystore (`setIsStrongBoxBacked(true)` where available), and gated
  by `setUserAuthenticationRequired(true)`. The seed is never exported, backed
  up, or displayed. Recovery is by rotation (section 3), never by seed restore.
- **Custodial keys are prohibited** for attesters.

## 2. Device requirements

| Requirement | Minimum |
| --- | --- |
| Android version | Android 10 (API 29) or later; Android 13 or later recommended |
| Keystore | Hardware-backed (TEE) required; StrongBox preferred |
| Key attestation | Android Key Attestation chain verified at enrollment, with `securityLevel` of `TrustedEnvironment` or `StrongBox` |
| Screen lock | PIN, pattern, password, or biometric required |
| Integrity | Play Integrity `MEETS_DEVICE_INTEGRITY` checked at enrollment and at most every 7 days when online. Failure blocks new attestations until re-verified; it does not suspend on-chain automatically, because connectivity is intermittent. |
| Rooted / unlocked bootloader | Not permitted |

## 3. Lifecycle procedures

### Enrollment

1. The CHW app generates the key **on-device**; the programme never sees private
   key material.
2. The app produces a key attestation certificate chain and a signed enrollment
   request (address plus attestation chain).
3. A programme supervisor verifies the CHW's identity in person, against a
   government ID or programme employment record, and records the binding
   `(CHW identity ↔ address)` off-chain.
4. The admin calls `add_attester_with_info`. The on-chain metadata carries no
   personal data (ADR-0001).

### Loss or theft

| Step | SLA |
| --- | --- |
| CHW or supervisor reports loss through the hotline or supervisor app | Immediately on discovery |
| Admin calls `suspend_attester` | **Within 4 hours** of report, 24/7 on-call |
| Determine the compromise start time (last known possession) | Within 24 hours |
| Mark compromise window from that time (`mark_attester_compromised`, #337) | Within 24 hours, once available |
| Enroll new device and rotate the attester key (#331), preserving identity | Within 72 hours |

Until #331 ships, rotation means: enroll the new address, `remove_attester` on
the old one, and record the old→new mapping off-chain.

### Offboarding

The CHW calls `renounce_attester` (#332) from their device. If the device is
unavailable, the admin calls `remove_attester`. Past attestations stay verifiable,
and `lafiya-web` displays the attester as removed (ADR-0006).

## 4. Shared devices

- **Several CHW keys on one device are allowed only in the interim model**, and
  only when each CHW has a separate Android user profile (multi-user). Each
  profile has its own keystore namespace and screen lock.
- Keys must never share an app profile or a user-authentication gate.
- Passkey (target) credentials are per-profile and device-bound, so the same
  rule applies.
- A shared device report of loss suspends **every** attester key on it.

## 5. Coercion (duress)

- The CHW app offers a **duress PIN**. Entering it opens the app normally, but
  the app silently submits an attester-initiated self-suspension (`self_suspend`,
  a follow-up contract feature) through the sponsored relayer, and quietly
  notifies the supervisor.
- Until `self_suspend` exists, the duress PIN sends a silent supervisor alert
  and the admin suspends under the loss/theft SLA.
- Reinstatement after duress is admin-only, after an in-person check.

## 6. Mapping to contract features

See the dependency table in [ADR-0012](../adr/0012-chw-attester-key-custody.md#contract-feature-dependencies).
Summary: rotation (#331, #355), compromise windows (#337), renouncement (#332),
sponsored attestations (#347), passkey signers (#356), and re-enrollment after
suspension (#324) are open; self-suspension for duress still has to be filed.

## Review

Field-operations review feedback is recorded in the pull request that
introduced this policy.
