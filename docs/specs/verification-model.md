# Verification-result model

- **Status:** Normative
- **Model version:** 1.0.0
- **Truth table:** [`verification-model.json`](verification-model.json)
- **Reference implementation:** [`scripts/verification_model.py`](../../scripts/verification_model.py)

The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** in this
document are to be interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

This spec defines what "verified" means for a Lafiya card. The contract views, the
TypeScript SDK, the CLI, `lafiya-web`, `lafiya-verifier`, and third-party verifiers
**MUST** implement this model, so that two verifiers never disagree about the same card
given the same inputs. Terms in *italics* are defined in the [glossary](../glossary.md).

## 1. Inputs

A verifier evaluates a card from the following inputs:

| Input | Source | Notes |
|---|---|---|
| Card payload | QR code / deep link | Includes the *record commitment* and the network the card was issued on. |
| Disclosed fields | Card holder | Used to recompute the *record commitment* ([LRC-1](../glossary.md#lrc-1-lafiya-record-commitment-v1)). |
| Chain state | `attestation-registry`, `attester-registry` | *Attestations* and history, revocation tombstones, *attester* lifecycle (active, *suspended*, removed), and attester compromise windows. |
| Verifier policy | Verifier configuration | Maximum attestation age; required attester status. |
| Trust anchors | Release manifest / `config/networks.toml` | The registry contract IDs and network passphrase the verifier trusts. |
| Current time | Verifier clock (or bundle time when offline) | Used for age and compromise-window checks. |

The truth table encodes these as the normalized boolean/enum fields in
`default_input` of `verification-model.json`. Implementations **MUST** reduce their raw
inputs to those fields before choosing a verdict.

## 2. Verdicts

| Verdict | Definition |
|---|---|
| `WrongNetwork` | The card was issued for a network (passphrase or registry contract ID) other than the verifier's trust anchors. |
| `Indeterminate` | A read that the verdict depends on failed (RPC error, timeout, or partial read). The card is neither accepted nor rejected. |
| `NotFound` | The chain was read successfully and no *attestation* exists for the *record commitment*. |
| `Revoked` | The latest attestation for the commitment has been revoked. |
| `AttesterCompromised` | The attestation was made inside a compromise window declared for its attester. |
| `AttesterRemoved` | The attester has been removed from the *allowlist*. |
| `Expired` | The attestation is older than the verifier's maximum-age policy. |
| `AttesterSuspended` | The attester is currently *suspended*. |
| `Superseded` | A newer attestation for the same record has replaced this one. |
| `Verified` | None of the above hold. |

Only `Verified` is a positive result. Implementations **MUST NOT** add verdicts outside
this list; they **MAY** attach extra diagnostic detail alongside a verdict.

An offline evaluation (from a signed bundle rather than a live read) **MUST** return the
same verdict as the online evaluation of the bundle's state, with the qualifier
`as_of_bundle`, and **MUST** show the bundle date to the user.

## 3. Precedence

When several conditions hold, the verdict is the first match in this order:

1. `WrongNetwork`: nothing else about the card can be trusted on the wrong network.
2. `Indeterminate` (attestation read failed): without the attestation there is nothing to judge.
3. `NotFound`
4. `Revoked`: an explicit, authoritative negative decision always wins over derived ones.
   It only needs the attestation read, so it is reported even when the attester read failed.
5. `Indeterminate` (attester status read failed): every verdict below depends on attester state.
6. `AttesterCompromised`: the attestation itself may be forged, which is worse than being stale.
7. `AttesterRemoved`
8. `Expired`: stale data is a stronger reason to reject than a temporary suspension.
9. `AttesterSuspended`: suspension may be lifted; it is reported but ranks below permanent conditions.
10. `Superseded`: the record was valid but a newer one exists.
11. `Verified`

## 4. Degraded modes

- **RPC failure:** if the attestation cannot be read, the verifier **MUST** return
  `Indeterminate` (unless `WrongNetwork` already applies).
- **Partial read failure:** a verifier **MUST NOT** return `Verified`, or any verdict
  ranked below position 5, when the attester status could not be read. It **MAY** still
  return `Revoked` or `NotFound`, which do not depend on attester state.
- **Offline mode:** see section 2. An offline verifier **MUST NOT** claim a live check.
- Implementations **MUST NOT** retry into a different network or registry to obtain a
  positive result.

## 5. Truth table

`verification-model.json` lists `default_input` and a set of `cases`. Each case overrides
some inputs and gives the required `verdict` (and `qualifier`, when present). Every
implementation **MUST** run the whole table in CI and fail on any mismatch.

| Implementation | Runs the table in CI |
|---|---|
| Reference (`scripts/verification_model.py`) | Yes (`ci.yml`, `docs-checks` job) |
| TypeScript SDK | Pending |
| `attestation-registry` `verify()` view | Pending (view not yet implemented) |
| Rust verifier | Pending |

## 6. Versioning

`model_version` in the truth table follows semantic versioning:

- **Major:** a verdict is added, removed, or renamed, or the precedence changes.
  Consumers **MUST** be updated before relying on the new version.
- **Minor:** new inputs or cases that do not change the verdict of any existing case.
- **Patch:** wording and clarifications only.

Implementations **SHOULD** report the model version they implement.
