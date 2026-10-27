# Runbook: Admin and Guardian Key Rotation

**Audience:** ceremony lead and on-call operator rotating a signer of the admin
or guardian `multisig-account`.

**Before you start:** the new key must come out of a ceremony. Run
[§3 to §4.5 of the key-generation ceremony](key-ceremony.md#4-procedure) for
the new seat(s). Never rotate in a key that was generated outside a
ceremony. For a suspected compromise, follow
[§5.2 of the ceremony runbook](key-ceremony.md#52-compromised-or-suspect-ceremony)
first. It decides whether rotation is enough.

## 1. Replace a signer

The account authorizes its own change, so `t` of the **current** signers must
sign. The retired key may be one of them.

1. Read the current state and compare it with the last ceremony record:

   ```bash
   scripts/verify_signer_set.py verify docs/security/ceremonies/<last>.json --source <G...>
   ```

   Do not continue unless every check passes.
2. Compute the new signer-set hash from the new key list, on two machines:

   ```bash
   scripts/verify_signer_set.py hash --threshold <t> <new G... list>
   ```

3. Build `set_signers(<new hex keys>, <t>)` on the account, collect `t`
   signatures with a [`@lafiya/multisig-auth`](../../packages/multisig-auth/README.md)
   ceremony file, and submit it. To change weights or roles, use
   `set_policy`. Then extend the record format and the `verify` policy check
   in the same PR.
4. Write the new record (`docs/security/ceremonies/<id>.json`, which names the
   record it replaces) and run `verify` against it. Then try one admin-gated
   call signed by the retired key plus another signer. It must fail with
   `UnknownSigner` (contract error `#5`).
5. Destroy the retired device and backup as in
   [§5.1](key-ceremony.md#51-planned-rotation).

## 2. Move admin to a new account

Use this when replacing the whole admin account, for example after a lost
quorum on an account whose keys may be compromised:

1. Run a full ceremony for the new account (deploy a new `multisig-account`).
2. On each registry, call `propose_admin(<new account>)`, signed by the old
   admin. Then call `accept_admin()`, signed by the new admin. The watchdog
   pages on `propose_admin`. Tell on-call before you start.
3. Update `config/networks.toml` and the record's `bindings`, then run
   `verify`.

## 3. Rotate the guardian

Replacing guardian signers works as in §1, on the guardian account. To
replace the guardian account itself: `grant_role(Guardian, <new>)`, then
`revoke_role(Guardian, <old>)`, both signed by admin. Run `verify` after each
step.

## 4. Rollback

`set_signers` takes effect immediately and is not timelocked. If the new
record fails `verify`, the current signers (old or new, whichever set holds
`t` honest keys) submit a corrective `set_signers` at once. Record the event
as a deviation in the transcript.
