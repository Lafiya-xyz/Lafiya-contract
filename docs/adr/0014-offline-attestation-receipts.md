<!-- adr-redirect: 0025-offline-attestation-receipts.md -->
# Moved: ADR-0025

- **Status:** Proposed
- **Date:** 2026-09-27
- **Deciders:** Lafiya contract maintainers

## Context

Emergencies happen where connectivity doesn't: rural roads, basements, crowded networks during
mass-casualty events. Today, Lafiya verification requires a live Soroban RPC call. Offline, a
responder has to choose between trusting an unverified card and ignoring it entirely.

[Issue #390](https://github.com/Lafiya-xyz/Lafiya-contract/issues/390) asks for an offline-
verifiable attestation receipt — a compact, signed statement that can be embedded in the QR code
and verified without a network connection, using a periodically synced trust bundle.

---

## Proposal

### Receipt structure

```rust
receipt = {
    v: 1,                         // receipt version
    network_id: [u8; 4],          // per ADR-0013 §6
    attestation_registry: [u8; 32], // contract ID
    record_hash: [u8; 32],        // the attested commitment (LRC-1 or LRC-2 root)
    attester: [u8; 32],           // attester Ed25519 public key
    attested_at: u64,             // Unix epoch seconds
    ledger_seq: u64,              // Stellar ledger sequence number
    attester_sig: [u8; 64],       // Ed25519 signature (see §canonical encoding)
}
```

### Canonical encoding for signature

The attester signs the following SHA-256 pre-image at attestation time:

```
canonical = SHA-256(
    "lafiya:receipt:v1\0"      // domain separator, NUL-terminated
    || network_id[4]
    || attestation_registry[32]
    || record_hash[32]
    || attester[32]
    || attested_at_BE[8]       // big-endian u64
    || ledger_seq_BE[8]        // big-endian u64
)
attester_sig = Ed25519Sign(attester_private_key, canonical)
```

This means the attester must sign the receipt when they are online (at attestation time), using
the same private key that corresponds to the `attester` public key registered in the
`attester-registry` on-chain. The receipt becomes part of the QR payload (key `7` in the CBOR
map, see ADR-0013 §4.3).

### Trust bundle

A **trust bundle** is a snapshot of allowlisted attester public keys, signed by the registry
admin's key, with an expiry timestamp.

```rust
trust_bundle = {
    v: 1,                         // bundle version
    network_id: [u8; 4],
    attestation_registry: [u8; 32],
    created_at: u64,              // Unix epoch seconds
    expires_at: u64,              // Unix epoch seconds
    attesters: Vec<[u8; 32]>,     // Ed25519 public keys of all active attesters
    admin_sig: [u8; 64],          // Ed25519 signature over canonical bundle encoding
}
```

The canonical encoding for the admin signature:

```
bundle_canonical = SHA-256(
    "lafiya:trust-bundle:v1\0"
    || network_id[4]
    || attestation_registry[32]
    || created_at_BE[8]
    || expires_at_BE[8]
    || u32_BE(len(attesters))
    || attester_0[32] || attester_1[32] || ... || attester_{n-1}[32]
)
admin_sig = Ed25519Sign(admin_private_key, bundle_canonical)
```

### Offline verification algorithm

```
function verify_offline(receipt, card_ref, trust_bundle, config):

  // 1. Bundle freshness
  now = current_unix_timestamp()
  if trust_bundle.expires_at < now:
    return reject(BUNDLE_EXPIRED)

  // 2. Network check
  if receipt.network_id != config.network_id:
    return reject(NETWORK_MISMATCH)
  if trust_bundle.network_id != config.network_id:
    return reject(NETWORK_MISMATCH)

  // 3. Registry check
  if receipt.attestation_registry != config.trusted_registry:
    return reject(UNTRUSTED_REGISTRY)

  // 4. Record hash matches QR card_ref
  if receipt.record_hash != card_ref:
    return reject(RECORD_HASH_MISMATCH)

  // 5. Attester in trust bundle
  if receipt.attester not in trust_bundle.attesters:
    return reject(ATTESTER_NOT_IN_BUNDLE)

  // 6. Receipt signature valid
  canonical = compute_receipt_canonical(receipt)
  if not Ed25519Verify(receipt.attester, canonical, receipt.attester_sig):
    return reject(INVALID_RECEIPT_SIGNATURE)

  // 7. Bundle admin signature valid
  bundle_canonical = compute_bundle_canonical(trust_bundle)
  if not Ed25519Verify(config.admin_pubkey, bundle_canonical, trust_bundle.admin_sig):
    return reject(INVALID_BUNDLE_SIGNATURE)

  return verified_offline(receipt.attested_at, trust_bundle.expires_at)
```

---

## Security analysis

### Offline window is a trust trade-off

The offline verification path introduces an inherent trust gap: **revocations that occur after
the bundle creation date are not visible to the offline verifier**. A responder who trusts a
receipt based on an expired bundle may verify a card for an attester who has since been suspended
or removed.

**Quantifying acceptable bundle lifetimes:**

| Bundle lifetime | Revocation visibility gap | Recommended use |
|-----------------|--------------------------|-----------------|
| ≤ 24 hours      | ≤ 24 h                   | High-value settings (hospitals with daily sync) |
| ≤ 7 days        | ≤ 7 days                 | Community health workers, weekly sync |
| ≤ 30 days       | ≤ 30 days                | Remote / low-connectivity settings |
| > 30 days       | Unacceptable             | Not recommended |

The UI MUST display "verified offline (as of [bundle date])" rather than a plain "verified"
indicator, clearly distinguishing offline verification from on-chain verification.

### Attacker-controlled receipt fields

The receipt is signed by the attester at attestation time. Once signed, the fields are
authenticated. An attacker cannot:
- Change `record_hash` without invalidating `attester_sig`.
- Substitute a different attester without invalidating `attester_sig`.
- Replay an old receipt for a different card (the `record_hash` binds it to the specific card).
- Forge a trust bundle (requires the admin private key).

### Bundle compromise

If the admin private key is compromised, an attacker could produce a trust bundle containing
rogue attester keys. Mitigations:
- The admin key should be the multisig account key, requiring N-of-M compromise.
- Bundle expiry limits the validity window of any forged bundle.
- Online reconciliation (§reconciliation) detects compromised bundles when connectivity returns.

### Revocation after bundle date

An attester whose key is in a valid bundle can produce receipts that offline verifiers will
accept, even after being suspended on-chain. This is an accepted trade-off at Lafiya's pre-alpha
stage. Production deployments should:
1. Keep bundle lifetimes short (≤ 7 days recommended).
2. Prompt users to sync when connectivity returns.
3. Display bundle age prominently in the offline verification UI.
4. Consider a CRL (Certificate Revocation List) embedded in the bundle for near-real-time
   revocation within the offline window.

### Replay attack

A signed receipt is valid for the specific `(network_id, attestation_registry, record_hash,
attester, attested_at, ledger_seq)` tuple. Replaying the same receipt for a different context
(different card, different network, different registry) is prevented by the canonical encoding
including all six fields.

### Freshness fields (`issued_at` / `valid_until`) and unreliable clocks

The QR payload (ADR-0013) carries optional `issued_at` and `valid_until` fields so that a
photographed card can be distinguished from a current one. The receipt's `attested_at` and
`ledger_seq` remain the authoritative on-chain anchor; the freshness fields are a verifier-side
policy input layered on top of the receipt.

**Canonical encoding.** When present, both fields are encoded as big-endian `u64` Unix epoch
seconds and are covered by the attester signature. They are appended to the receipt pre-image
after `ledger_seq_BE[8]`, guarded by a presence bitmap so that a payload with neither field
produces a byte-identical canonical form to the v1 receipt:

```
canonical = SHA-256(
    "lafiya:receipt:v1\0"
    || network_id[4]
    || attestation_registry[32]
    || record_hash[32]
    || attester[32]
    || attested_at_BE[8]
    || ledger_seq_BE[8]
    || freshness_flags[1]        // bit 0 = issued_at present, bit 1 = valid_until present
    || issued_at_BE[8]?          // only if bit 0 set
    || valid_until_BE[8]?        // only if bit 1 set
)
```

Because the flags byte is always present, a v1 receipt (no freshness fields) hashes with
`freshness_flags = 0x00`. This is an additive extension under the payload versioning rules in
ADR-0013: the payload version is not bumped, and verifiers that predate this extension ignore
the unknown fields but still validate the signature over the extended pre-image only if they
understand the flags byte. To keep old verifiers working, the extension is gated behind a
payload minor-version marker (see ADR-0013 §versioning); a verifier that does not recognize the
marker MUST treat the payload as unverifiable rather than silently ignoring the fields.

**Verifier policy.** Staleness handling is configurable per deployment with three levels:

| Policy | Behaviour when `now > valid_until` (or `now < issued_at`) | Recommended use |
|--------|-----------------------------------------------------------|-----------------|
| `warn` | Accept, but surface a "stale card" warning and the field values | Low-risk triage, training |
| `soft-fail` | Accept only after explicit responder confirmation; default UI is "unverified" | Field responders (default) |
| `hard-fail` | Reject; treat as `STALE_PAYLOAD` | High-value settings, hospitals |

The default for responder deployments is `soft-fail`. Missing fields are not an error: a payload
without `issued_at`/`valid_until` is treated as "freshness unknown" and follows the same policy
as a stale payload (`warn` → warn, `soft-fail` → confirm, `hard-fail` → reject), so that
stripping the fields cannot downgrade enforcement.

**Unreliable clocks.** Offline verifiers may have no trustworthy wall clock (device clock drift,
no NTP, factory-reset devices). The verifier MUST NOT trust its own clock as the sole source of
truth for staleness:

1. If the device clock is known to be unreliable (no recent network sync, or the clock is
   outside the trust bundle's `[created_at, expires_at]` window), the verifier downgrades
   `hard-fail` to `soft-fail` and displays "freshness unverified (clock unreliable)".
2. The trust bundle's `expires_at` is the primary freshness anchor: a receipt is never accepted
   past `trust_bundle.expires_at` regardless of `valid_until`, so an unreliable clock cannot
   extend acceptance beyond the bundle window.
3. When the device clock is behind the bundle's `created_at`, the verifier uses
   `trust_bundle.created_at` as a floor for "now" so that a rolled-back clock cannot make an
   expired payload appear current.
4. Once connectivity returns, the online reconciliation step re-evaluates freshness against the
   authoritative chain time and upgrades or downgrades the status accordingly.

This means the freshness fields tighten the offline window but never widen it: the effective
acceptance window is `min(valid_until, trust_bundle.expires_at)` measured against a clock that
is clamped to the bundle's validity range.

### Cross-network confusion

`network_id` is included in both the receipt and the trust bundle, and checked against the
verifier's configured network. A testnet receipt cannot verify against a mainnet trust bundle
because the `network_id` fields will differ.

---

## CLI commands

The `lafiya-cli` crate provides commands for trust-bundle management:

```bash
# Generate a trust bundle from the on-chain attester list
lafiya-cli trust-bundle generate \
  --network testnet \
  --attestation-registry <CONTRACT_ID> \
  --admin-key <KEY_IDENTITY> \
  --valid-days 7 \
  --output trust-bundle.json

# Verify a trust bundle signature
lafiya-cli trust-bundle verify \
  --bundle trust-bundle.json \
  --admin-pubkey <HEX_PUBKEY> \
  --network testnet

# Show bundle contents
lafiya-cli trust-bundle show \
  --bundle trust-bundle.json
```

These commands are implemented in `crates/lafiya-cli/src/main.rs` under the `TrustBundle`
subcommand (see Issue #390).

---

## Online reconciliation

When connectivity returns, the verifier app should:

1. Check the receipt's `record_hash` against `attestation-registry.get_attestation`.
2. Check that `receipt.attester` is still allowlisted in `attester-registry.is_attester`.
3. If the attestation has been revoked or the attester suspended, flag the card and notify
   the responder.

This reconciliation step converts an "offline verified" status to a "chain-verified" or
"revocation detected" status. It should happen automatically in the background when connectivity
is first detected after an offline verification.

---

## Implementation

The receipt and trust-bundle codecs are implemented in `crates/lafiya-qr/src/lib.rs` as the
[`OfflineReceipt`] struct and encoding (see ADR-0013 §4.3 for how the receipt is embedded in
the QR payload). The CLI trust-bundle commands are in `crates/lafiya-cli/src/main.rs`.

Test vectors for receipts and trust bundles are in `crates/lafiya-qr/vectors/`.

---

## Consequences

### Positive
- Responders in low-connectivity environments can verify cards without a live RPC call.
- The attester signature at attestation time binds the receipt to the specific card and attester.
- The trust bundle's admin signature prevents forged bundles.
- The offline path degrades gracefully to an "unverified" state when the bundle expires.

### Trade-offs and risks
- Revocations within the bundle lifetime window are not detected offline (accepted trade-off;
  mitigated by short bundle lifetimes and the online reconciliation step).
- The attester must be online at attestation time to sign the receipt (this is already required
  to submit the on-chain attestation, so it adds no new requirement).
- Trust bundle management adds operational overhead for the admin (generating and distributing
  bundles, rotating after key compromise).

## References

- [ADR-0008](0008-record-commitment-canonicalization.md) — LRC-1
- [ADR-0012](0012-lrc2-selective-disclosure.md) — LRC-2
- [ADR-0013/qr-payload-v1.md](../specs/qr-payload-v1.md) — QR payload spec
- [Issue #390](https://github.com/Lafiya-xyz/Lafiya-contract/issues/390)
- [`crates/lafiya-qr/src/lib.rs`](../../crates/lafiya-qr/src/lib.rs) — OfflineReceipt codec
- [`crates/lafiya-cli/src/main.rs`](../../crates/lafiya-cli/src/main.rs) — CLI trust-bundle commands
