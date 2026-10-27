# Verification-result model

- **Status:** Normative
- **Model version:** 1.1.0
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
| `Revoked` | The latest attestation for the commitment has been revoked, or the commitment is set in the status list (section 8). |
| `AttesterCompromised` | The attestation was made inside a compromise window declared for its attester. |
| `AttesterRemoved` | The attester has been removed from the *allowlist*. |
| `Expired` | The attestation is older than the verifier's maximum-age policy. |
| `AttesterSuspended` | The attester is currently *suspended*. |
| `Superseded` | A newer attestation for the same record has replaced this one. |
| `Disputed` | A patient-signed *dispute* is active for the commitment (section 7). |
| `QuorumNotMet` | The record class requires N distinct attesters and fewer than N valid co-attestations exist (section 6). |
| `Verified` | None of the above hold. |

Only `Verified` is a positive result. Implementations **MUST NOT** add verdicts outside
this list; they **MAY** attach extra diagnostic detail alongside a verdict.

`Verified` **MUST** additionally report quorum satisfaction (section 6): the number of
distinct valid attesters, the policy threshold, and whether the policy is satisfied.
A `Verified` result whose quorum policy is not satisfied is not permitted; such a card
**MUST** be reported as `QuorumNotMet`.

An offline evaluation (from a signed bundle rather than a live read) **MUST** return the
same verdict as the online evaluation of the bundle's state, with the qualifier
`as_of_bundle`, and **MUST** show the bundle date to the user.

### 2.1 Annotations

Verifiers **MUST** display, and **MUST NOT** use to change the verdict, the following
annotations. They describe *who* attested and under what authority; every verdict-changing
condition is already covered by the attester status inputs above. The truth table lists them
in `annotations` and includes cases proving they leave the verdict unchanged.

| Annotation | Source | Display |
|---|---|---|
| `attester_tier` | `attester-registry.get_attester_tier` | `CHW`, `Nurse` or `Physician`. Unassigned attesters show `CHW`. |
| `attestation_class` | `attestation-registry.get_attestation_scope` | The class the attester attested under, if any. |
| `attester_on_probation` | `attester-registry.get_probation_state` | Show a "new attester" notice while `Probation` or `Flagged`. |
| `facility_id` | `attestation-registry.get_attestation_scope` | The facility named at attestation time, if any. |
| `facility_affiliated_at_attestation` | `facility-registry.was_affiliated_at` | Evaluated at the attestation timestamp, so later disaffiliation does not hide it. |
| `facility_active` | `facility-registry.is_facility_active` | Show a warning when a named facility is deactivated. |

Tier and class are enforced when the attestation is made (`attest_with_scope`), so a
later capability-matrix change does not invalidate existing attestations; verifiers show
the attester's *current* tier and never re-derive class eligibility.

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
11. `Disputed`: the data is present and not otherwise invalid, but the patient contests it.
    It ranks below `Superseded` because a superseding record resolves the dispute.
12. `QuorumNotMet`: the record exists and is individually valid but lacks the required
    number of distinct attesters.
13. `Verified`

## 4. Degraded modes

- **RPC failure:** if the attestation cannot be read, the verifier **MUST** return
  `Indeterminate` (unless `WrongNetwork` already applies).
- **Partial read failure:** a verifier **MUST NOT** return `Verified`, or any verdict
  ranked below position 5, when the attester status could not be read. It **MAY** still
  return `Revoked` or `NotFound`, which do not depend on attester state.
- **Offline mode:** see section 2. An offline verifier **MUST NOT** claim a live check.
- Implementations **MUST NOT** retry into a different network or registry to obtain a
  positive result.

## 6. Quorum policy

A record class **MAY** declare a *quorum policy*: a minimum number `N` of **distinct**
attesters whose valid attestations for the same *record commitment* are required before
the record is reported as `Verified`. High-assurance classes (for example genotype)
**SHOULD** set `N >= 2`. The policy is stored per record class in `attestation-registry`
and is public.

Rules:

- **Distinct-attester rule.** Co-attestations are aggregated by attester identity. A
  second attestation from an attester that already attested the commitment **MUST NOT**
  increase the count; the contract **MUST** treat it as a no-op (it **MAY** return an
  error instead, but **MUST NOT** count it twice).
- **Validity of a co-attestation.** A co-attestation counts toward quorum only if, on
  its own, it would not yield any verdict ranked above `QuorumNotMet` in section 3
  (i.e. it is not revoked, compromised, removed, expired, suspended, or superseded).
- **Monotonicity under revocation.** Revoking any single co-attestation **MUST NOT**
  increase the distinct-attester count. The count is non-increasing as attestations are
  revoked; a verifier **MUST** re-evaluate quorum after applying revocations.
- **Supersession.** Supersession applies per record, not per attester: when a newer
  attestation supersedes the record, the quorum is evaluated over the attesters of the
  current (non-superseded) attestation set only.
- **Privacy.** The verification view **MUST** return only the count, the threshold, and
  the satisfaction boolean. It **MUST NOT** reveal which attesters contributed beyond
  what is already public on chain.

## 7. Disputed overlay

A patient (the holder of the card) **MAY** flag an attestation as disputed by submitting
a signature over the *record commitment* using the patient signature scheme defined in
the QR spec. The dispute is stored as a `Disputed` overlay keyed by (record hash, signer)
and **MUST NOT** delete or mutate the underlying attestation.

- **Domain separation.** The signed message **MUST** be domain-separated from all other
  patient signatures (e.g. a distinct prefix/tag and the registry contract ID and network
  passphrase), so a dispute signature cannot be replayed as any other authorization and
  vice versa. Dispute signatures **MUST** be replay-safe: a replayed submission is a
  no-op.
- **Clearing.** Only an attester or an admin **MAY** clear a dispute. The patient cannot
  clear or delete it.
- **Anti-abuse.** The number of active disputes per signer and per commitment **MUST** be
  bounded; submissions beyond the bound are rejected.
- **Responder guidance.** When a verifier returns `Disputed`, it **MUST** surface the
  state to the user and **SHOULD** advise re-checking the disclosed fields with the
  issuing facility; it **MUST NOT** present the card as verified.
- **Privacy note.** A dispute is a linkability signal: it ties a patient key to a
  commitment on chain. This trade-off is documented in the privacy review.

## 8. Status-list revocation checks

For offline and bandwidth-constrained verifiers, revocation **MAY** be checked against a
*status list* (a bitstring) instead of per-record lookups. The status list follows W3C
Bitstring Status List semantics where practical.

- **Index allocation.** Each commitment is assigned a status-list index that does not
  reveal record identity (indices are allocated without deriving them from the record
  hash in a reversible way).
- **Versioned segments.** The contract stores segments with versioned commitments; a
  verifier validates inclusion against a signed root.
- **Interplay with verdicts.** A set bit for a commitment's index yields `Revoked` at the
  same precedence as a per-record revocation (position 4 in section 3). A status list
  that cannot be validated (bad signature, stale root) yields `Indeterminate`.
- **Supersession.** Status-list bits track revocation only; supersession is still
  determined from the attestation set.

## 5. Truth table

`verification-model.json` lists `default_input` and a set of `cases`. Each case overrides
some inputs and gives the required `verdict` (and `qualifier`, when present). Every
implementation **MUST** run the whole table in CI and fail on any mismatch.

| Implementation | Runs the table in CI |
|---|---|
| Reference (`scripts/verification_model.py`) | Yes (`ci.yml`, `docs-checks` job) |
| TypeScript SDK | Pending (must also cover `Disputed` and quorum fields) |
| `attestation-registry` `verify()` view | Pending (view not yet implemented) |
| Rust (`crates/lafiya-verification-model`) | Yes (`cargo test`) |

The truth table **MUST** include cases for quorum satisfaction and non-satisfaction, the
distinct-attester no-op rule, monotonicity of the count under revocation, the `Disputed`
state and its precedence, and status-list-based `Revoked`/`Indeterminate` results.

## 6. Versioning

`model_version` in the truth table follows semantic versioning:

- **Major:** a verdict is added, removed, or renamed, or the precedence changes.
  Consumers **MUST** be updated before relying on the new version.
- **Minor:** new inputs or cases that do not change the verdict of any existing case.
- **Patch:** wording and clarifications only.

Implementations **SHOULD** report the model version they implement.

## 7. Differential testing

The truth table pins chosen cases; it cannot show that two implementations agree
everywhere else. `scripts/differential_verification.py` runs the reference, the
Rust crate (`lafiya-verification-model`, JSON lines on stdin/stdout), and the
TypeScript SDK (`packages/verifier-sdk/scripts/model-eval.mjs`) over generated
scenarios and compares verdict, `reason` (the precedence rule that matched), and
qualifier:

- **Generator.** Input domains come from this model: booleans take both values,
  enum inputs take every value appearing in `default_input` or any case. Every
  case, the whole input space (while it has at most 65,536 points), and 500
  seeded random samples per seed are evaluated.
- **On every PR** (`ci.yml`, `differential` job) with the fixed seeds `1..5`;
  **nightly** (`differential-nightly.yml`) with a fresh random seed printed in
  the log and job summary.
- **Failure.** Any discrepancy fails the job and prints a minimal reproducing
  scenario: the inputs are greedily reset to `default_input` while the
  implementations still disagree, leaving only the overrides that matter, in
  the shape of a `cases[].input` entry.
- **Coverage.** The number of scenarios per verdict and per reason is printed
  (and written to the GitHub job summary); a verdict that is never reached fails
  the run.

### Extending the model

When adding an input, an enum value, or a verdict:

1. Update `verification-model.json` (`default_input`, `verdicts`, and at least
   one case exercising each new value, which is also what puts a new enum
   value into the generator's domain) and bump `model_version` (section 6).
2. Update `explain()` in `scripts/verification_model.py`, then mirror it in
   `crates/lafiya-verification-model/src/lib.rs` (`Input`, `Verdict`,
   `Reason`, `evaluate`) and `packages/verifier-sdk/src/model.ts`
   (`ModelInput`, `MODEL_VERDICTS`, `ModelReason`, `evaluateModel`), and the
   `MODEL_VERSION` constants. The Rust `Input` rejects unknown fields, so a
   forgotten input fails loudly.
3. Run `python3 scripts/verification_model.py`, `cargo test -p
   lafiya-verification-model`, `pnpm test` in `packages/verifier-sdk`, then
   `pnpm build` there and `python3 scripts/differential_verification.py`.
4. If the input space grows past `--max-exhaustive`, the run falls back to
   seeded sampling; raise `--samples` in CI so every verdict stays covered.
