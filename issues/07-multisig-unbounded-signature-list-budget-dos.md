---
title: "[audit] SEC-01: MultisigAccount::__check_auth has no upper bound on signature count"
labels: bug, priority:p1, security, architecture, severity:high
---

**Severity:** High
**Difficulty:** Low — requires only constructing an oversized `Vec<Signature>` in the authorization entry; no privileged access needed
**Type:** Denial of Service / Unbounded Resource Consumption (CWE-400)

> This is the auth check the protocol invokes on every transaction the
> account authorizes, per ADR-0003's intended production admin-custody
> model — a resource-exhaustion vector here has account-wide blast
> radius, not just a single call's.

## Summary

`MultisigAccount::__check_auth` bounds the *minimum* number of signatures
required (`>= threshold`) but never bounds the *maximum*. The loop that
performs per-signature storage lookups and `ed25519_verify` calls is
indexed by `signatures.len()` — a value fully controlled by whoever
constructs the authorization entry, with no contract-enforced ceiling
tied to the account's actual configuration.

## Location

`contracts/multisig-account/src/lib.rs:65-106`

## Technical Detail

```rust
65	    fn __check_auth(
66	        env: Env,
67	        signature_payload: Hash<32>,
68	        signatures: Self::Signature,
69	        _auth_contexts: Vec<Context>,
70	    ) -> Result<(), Error> {
71	        let threshold: u32 = env
72	            .storage()
73	            .instance()
74	            .get(&DataKey::Threshold)
75	            .ok_or(Error::NotInitialized)?;
76	
77	        if signatures.len() < threshold {
78	            return Err(Error::NotEnoughSigners);
79	        }
80	
81	        for index in 0..signatures.len() {
82	            let signature = signatures.get_unchecked(index);
83	            if index > 0 {
84	                let previous = signatures.get_unchecked(index - 1);
85	                if previous.public_key >= signature.public_key {
86	                    return Err(Error::BadSignatureOrder);
87	                }
88	            }
89	
90	            if !env
91	                .storage()
92	                .instance()
93	                .has(&DataKey::Signer(signature.public_key.clone()))
94	            {
95	                return Err(Error::UnknownSigner);
96	            }
97	
98	            env.crypto().ed25519_verify(
99	                &signature.public_key,
100	                &signature_payload.clone().into(),
101	                &signature.signature,
102	            );
103	        }
104	
105	        Ok(())
106	    }
```

`signatures: Vec<Signature>` originates from the caller-supplied
`SorobanCredentials::Address` authorization entry — it is not validated
against the account's configured signer set before `__check_auth` runs.
The strict-ascending-order check at lines 84-87 prevents literal
duplicate entries for the same public key, but places no ceiling on the
list's total length. The per-entry unknown-signer check (lines 90-96)
runs before the crypto call for that entry, so the check order limits
*which* entries reach `ed25519_verify` — but it does not bound how many
entries the loop processes overall: the loop body runs once per element
of `signatures`, and `signatures.len()` is attacker-controlled with no
upper limit.

## Impact

The cost of a single `__check_auth` invocation scales with attacker
input, not with the account's configured `threshold` or signer count. A
caller can pad the authorization entry with an arbitrarily long
`Vec<Signature>`, forcing the account contract to iterate, perform
storage lookups, and — for any entries whose `public_key` matches a real
configured signer — perform genuine `ed25519_verify` calls, an expensive
operation, for far more entries than a legitimate `threshold`-sized
submission would ever require. Against a resource-metered execution
environment, this is a budget-griefing vector: a submission cheap for the
attacker to construct forces disproportionate, attacker-directed cost
onto the account's authorization path, either wasting resources relative
to a legitimate call or pushing the transaction toward the network's
hard resource ceiling.

## Recommendation

Store the configured signer count at `__constructor` time (or derive it
from `signers.len()` at construction and persist it alongside
`Threshold`), and reject oversized lists before any per-entry work:

```rust
if signatures.len() > signer_count {
    return Err(Error::TooManySigners); // new Error variant
}
```

Place this check immediately after the existing `signatures.len() <
threshold` check, before the loop. This bounds worst-case cost to
`O(signer_count)` regardless of caller input, since supplying more
signatures than there are configured signers can never be legitimate.

## Verification

- [ ] `__check_auth` rejects any `signatures` list longer than the
      account's configured signer count, before any per-entry storage
      lookup or crypto verification executes.
- [ ] New `Error::TooManySigners` (or equivalent) variant added, tested,
      and documented in `docs/error-codes.md` (see QA-01 — that document
      currently has no `multisig-account` section at all).
- [ ] `contracts/multisig-account/src/test.rs` gains a test submitting
      more signatures than configured signers and asserting rejection.
