# Ceremony transcript: 2026-10-18 testnet dry run

| | |
| --- | --- |
| Ceremony id | `2026-10-18-testnet-dry-run` |
| Network | testnet (`Test SDF Network ; September 2015`) |
| Procedure | [Key-generation ceremony](../../runbooks/key-ceremony.md), with the deviations below |
| Record | [`2026-10-18-testnet-dry-run.json`](2026-10-18-testnet-dry-run.json) |
| Purpose | Exercise the whole flow on chain: generation, multisig deployment, registry wiring, guardian grant, one signer rotation, and verification. |

This transcript holds public data only. Every secret key used here was
throwaway, existed only for this run, and was deleted when it ended. None of
these keys or contracts may be used for anything else.

## Deviations from the procedure (dry run only)

| Procedure step | What the dry run did instead |
| --- | --- |
| §1 Separate holders and witnesses | One operator played every seat. There were no independent witnesses. |
| §2–3 Hardware wallets and device checks | Keys came from `stellar keys generate` (stellar-cli 25.2.0, OS CSPRNG) in a throwaway config directory. No hardware devices were used. |
| §4.4 Seed backups | None. The keys were discarded after the run. |
| §4.7 `@lafiya/multisig-auth` ceremony files | No Node runtime was available. Auth entries were signed with `stellar-sdk` (Python) 13.1.0 in the same format: a `Vec<Signature { public_key: SignerKey::Ed25519, signature }>` over the Soroban authorization payload, sorted by key. |

## Seats

| Account | Seat | Public key |
| --- | --- | --- |
| admin | admin-1 | `GAFVRWTIY2V5XYYWTRSXHBNPUPAGDZGO65R4YAFFWO3ZALZIQRYAY2BC` |
| admin | admin-2 | `GBG7DPYBGYY52QTTA4HON4Y3KZLHQ33RFAUNHCMYTRJ63RP337IAYYGX` |
| admin | admin-3 (rotated out) | `GCZFTV6RJVEAZDO2E25SODXWXQMGYVCO2YKGQMFQVCO3KE6AZLERGKEP` |
| admin | admin-4 (rotated in) | `GA64WEBD66O5DAIZ3C5WRCSAXCHUSONX76MEGOPIAUZJABQPHOOWF7M3` |
| guardian | guardian-1 | `GBD46NUEETSNCNPXSRNXLRNZ5THX6CLBNSVAER32RC4ZUY66OLIHSM7A` |
| guardian | guardian-2 | `GD5QG446KYZVAYA6NLSOFK7USKGNIYL43V43XGTBPMHRIICMWZR2CPWR` |

A throwaway deployer, `GCSWY2KG3QV4PNPPMNVUJXOL5Q3NJ7WXAURE2WFGNASEQZR66B2MT6OR`,
paid the fees. It holds no authority over any contract.

## Signer-set hashes (§4.5)

| Account | Threshold | Keys | Hash |
| --- | --- | --- | --- |
| admin (initial) | 2 | admin-1, admin-2, admin-3 | `ae7a9dc287e1e602dd2b9c2978aca84143691d0af3c3c7b277c73c67f261fee9` |
| admin (after rotation) | 2 | admin-1, admin-2, admin-4 | `6db57bd3676fe8c186fbd42f84351ed49682c0759d7937dd02c371b07aa930bc` |
| guardian | 1 | guardian-1, guardian-2 | `0a7f3ceba24e8e3ae02d7fb23dbd6ac157ba31144fe1c8b89e5ce8fd07bb1ea5` |

## On-chain steps (§4.7)

Contracts were built with `stellar contract build` from `dd36b06` (`main`).
All times are UTC.

| Time | Step | Transaction |
| --- | --- | --- |
| 09:33:42 | Fund deployer (friendbot) | `29517a75f2ef72ee0df406cb608da6474671ec7a87c167c88d97b137310238c5` |
| 09:38:02 | Upload `multisig_account.wasm` (`d192d238…e36031`) | `94ae0c55419ae407b0fb81d591658ccfed46c50659cef95d2db6a69eb75f405a` |
| 09:38:07 | Upload `attester_registry.wasm` (`13a69dea…7cffee`) | `154f920af831d2ed61ee27d54c2c332763c49451d6cfb2a3ee5b8ed48c9cd91b` |
| 09:38:17 | Upload `attestation_registry.wasm` (`1e767c96…5b5522`) | `2650b30d6b5e5a1e9648320f80ce0e9d7de5882e1f9446507334153dc500b314` |
| 09:39:02 | Deploy admin `multisig-account` 2-of-3 → `CD77H3QJIVZ4NBZSWEV5DR6S4JAVTZZ3VUZTNUNASIVBFLF27BZ4COJM` | `aafa98cc0e601440ea7bac4a287bf13cf92f07577c5dbc10dacdf79eb53e9761` |
| 09:39:07 | Deploy guardian `multisig-account` 1-of-2 → `CCMFO2SONSO5GNHK2ZLC52VFIRW3KCEASAKNQ5ER2VJWWBD6XM7HE5DA` | `ed67da7ebc361e4976225fbcf340e2411a1df7f9ea7066bd9162fcd9303d41a4` |
| 09:39:57 | Deploy `attester-registry` with admin = admin account (constructor signed by admin-1 and admin-2) → `CB3IK6IXEAYOBWZ4KO5IVYPW75X3CFSK3VPHKGKWYEJFALCQWYEAPKAI` | `12b5f9a879a2803d3c62800aa5ec284856984b1fda03b52b92c0d4d6fbb11eb7` |
| 09:40:22 | Deploy `attestation-registry` with admin = admin account (signed by admin-1 and admin-2) → `CBPF6QCJVZ3OV7HJQ5LQWIZD4I3UJJIYD7FWJUUZ3QYNLM35T5CJDOFG` | `b163a00d8585d1c943fa6ad9e879a21023916ad58d444e014c85d18f666c50a9` |
| 09:40:27 | `attester-registry.grant_role(Guardian, guardian account)` (admin-1 and admin-2) | `834aeb4566265935d5a1b8cba96bb54e26f6e6798bd8c11e518f1cefad4f9139` |
| 09:40:32 | `attestation-registry.grant_role(Guardian, guardian account)` (admin-1 and admin-2) | `2ecd609ab8d5ce62de84fec2de1409fcf58fac38e5986c4f2694d7f60df11649` |
| 09:43:12 | Rotation ([admin key rotation](../../runbooks/admin-key-rotation.md) §1): admin `set_signers([admin-1, admin-2, admin-4], 2)` signed by admin-1 and admin-2 | `a370e2d1d08330666401409fa17121763a2ea7c6eb44a86a7da0086d9f5a0227` |

## Retired key rejected (rotation step 4)

The operator simulated `attester-registry.set_max_attesters(500)` signed by
admin-1 and the retired admin-3. The admin account's `__check_auth` rejected
it with `Error(Contract, #5)` (`UnknownSigner`). Nothing was submitted.

## Verification (§4.8)

```text
$ scripts/verify_signer_set.py verify docs/security/ceremonies/2026-10-18-testnet-dry-run.json --source deployer
ceremony 2026-10-18-testnet-dry-run on testnet
[PASS] admin: no duplicate keys
[PASS] admin: threshold 2-of-3 within policy (2 <= t < n)
[PASS] admin: recorded signer-set hash matches keys
[PASS] guardian: no duplicate keys
[PASS] guardian: threshold 1-of-2 within policy (1 <= t < n)
[PASS] guardian: recorded signer-set hash matches keys
[PASS] admin and guardian share no keys
[PASS] admin: on-chain signer set equals the ceremony record
[PASS] admin: on-chain threshold is 2
[PASS] admin: policy is plain 2-of-n (no weights or roles)
[PASS] admin: signer-set hash recomputed from chain matches
[PASS] guardian: on-chain signer set equals the ceremony record
[PASS] guardian: on-chain threshold is 1
[PASS] guardian: policy is plain 1-of-n (no weights or roles)
[PASS] guardian: signer-set hash recomputed from chain matches
[PASS] attester-registry: admin is the admin account
[PASS] attester-registry: guardian account holds Guardian
[PASS] attestation-registry: admin is the admin account
[PASS] attestation-registry: guardian account holds Guardian
all checks passed
```

The same command was then run against a stale copy of the record that still
listed admin-3, with the pre-rotation hash. It exited 1 with:

```text
[FAIL] admin: on-chain signer set equals the ceremony record: missing ['GCZFTV6RJVEAZDO2E25SODXWXQMGYVCO2YKGQMFQVCO3KE6AZLERGKEP'], unexpected ['GA64WEBD66O5DAIZ3C5WRCSAXCHUSONX76MEGOPIAUZJABQPHOOWF7M3']
[FAIL] admin: signer-set hash recomputed from chain matches
2 check(s) failed
```

## Close

All throwaway secret keys and the CLI config directory were deleted after
verification. Testnet resets will remove these contracts. The record stays
as an example of the format.
