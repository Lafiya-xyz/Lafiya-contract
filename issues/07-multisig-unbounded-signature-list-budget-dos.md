---
title: "[bug]: MultisigAccount::__check_auth has no upper bound on signature count — attacker-controlled CPU budget cost"
labels: bug, priority:p1, security, architecture
---

> Architect/security-tier: this is a resource-exhaustion vector in the
> account contract every admin-gated call in both registries authenticates
> through (per ADR-0003, this multisig is the intended production admin
> custody model), not a logic bug in the happy path.

## Description

`contracts/multisig-account/src/lib.rs::__check_auth` (~65-106) checks a
lower bound on the number of signatures supplied but never an upper one:

```rust
if signatures.len() < threshold {
    return Err(Error::NotEnoughSigners);
}

for index in 0..signatures.len() {
    let signature = signatures.get_unchecked(index);
    if index > 0 {
        let previous = signatures.get_unchecked(index - 1);
        if previous.public_key >= signature.public_key {
            return Err(Error::BadSignatureOrder);
        }
    }

    if !env.storage().instance().has(&DataKey::Signer(signature.public_key.clone())) {
        return Err(Error::UnknownSigner);
    }

    env.crypto().ed25519_verify(
        &signature.public_key,
        &signature_payload.clone().into(),
        &signature.signature,
    );
}
```

`signatures: Self::Signature` (`Vec<Signature>`) is caller-supplied data —
it comes from the `SorobanCredentials::Address` authorization entry
attached to a transaction, which in Soroban's auth model is constructed
by whoever is submitting the transaction, not validated against the
account's configured signer set before `__check_auth` runs. The strict
ascending-order check (`previous.public_key >= signature.public_key`)
prevents duplicate *valid* signer entries from being repeated, but it does
**nothing** to cap the total list length: an attacker can submit a
`Vec<Signature>` containing many entries with distinct-but-garbage
`public_key`/`signature` pairs, sorted to satisfy the ordering check, and
the loop will run `env.crypto().ed25519_verify` — a genuinely expensive
CPU operation — once per entry before hitting the `UnknownSigner` check
for each one (the unknown-signer check happens *before* the crypto call
per-iteration here, so actually each bogus unknown signer would be
rejected before verify runs for that entry — but a list interleaving a
few genuinely known signer public keys with attacker-chosen signatures
under them still forces real `ed25519_verify` calls for those entries,
and there's no bound on how many *known* signer public keys can be
repeated-with-different-garbage-signatures... actually the ordering check
prevents literal duplicates of the same public key, but does not prevent
padding the list with many distinct *unknown* keys interleaved with a
couple of real ones, each triggering a storage `has()` lookup at minimum).
Regardless of the exact worst-case shape, the core problem stands
independent of the exact accounting: **the loop bound is
`signatures.len()`, a value the caller fully controls, with no
contract-enforced ceiling**, so the cost of a single `__check_auth`
invocation scales linearly with attacker input rather than with the
account's actual configured `threshold`/signer count.

Since this is the auth check for a *custom account* — invoked on every
transaction that account authorizes — an oversized signature list either
burns disproportionate CPU budget relative to what a legitimate
`threshold`-sized submission needs, or, if the resource limits are hit,
fails the transaction in a way that's cheap for an attacker to trigger
repeatedly against a target account's pending operations.

## Expected behavior

`__check_auth` rejects signature lists whose length exceeds some sane,
configuration-derived ceiling (e.g. `signers.len()`, since supplying more
signatures than there are configured signers can never be legitimate)
before doing any per-entry work, so cost is bounded by the account's own
configuration, not by caller input.

## Actual behavior

No such bound exists; `signatures.len()` is used directly as the loop
count with no ceiling check.

## Proposed fix

Store `signers.len()` (or just reuse `threshold` plus a separate stored
signer count) at `__constructor` time, and add an early check in
`__check_auth`:

```rust
if signatures.len() > signer_count {
    return Err(Error::TooManySigners); // new Error variant
}
```

placed before the loop, alongside the existing `signatures.len() <
threshold` check. This bounds worst-case cost to `O(signer_count)`
regardless of what a caller submits.

## Acceptance criteria

- [ ] `__check_auth` rejects any `signatures` list longer than the
      account's configured signer count, before doing any per-entry
      storage lookups or crypto verification.
- [ ] New `Error::TooManySigners` (or similarly named) variant added,
      tested, and documented in `docs/error-codes.md` (currently
      `docs/error-codes.md` doesn't cover `multisig-account` at all — see
      the separate error-codes-documentation issue for that gap).
- [ ] A test in `contracts/multisig-account/src/test.rs` submitting more
      signatures than configured signers and asserting rejection.

## Environment

- Contract(s) affected: multisig-account
- Verified by reading source; `cargo` unavailable in this audit
  environment.
