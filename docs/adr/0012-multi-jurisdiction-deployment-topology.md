<!-- adr-redirect: 0020-multi-jurisdiction-deployment-topology.md -->
# Moved: ADR-0020

- **Status:** Proposed
- **Date:** 2026-09-25
- **Deciders:** Lafiya contract maintainers

> **Terminology:** *jurisdiction*, *federation*, *trust list*, *attester*, and *admin* are
> defined in the [glossary](../glossary.md).

## Context

Lafiya is framed as a Digital Public Good, so we expect it to be adopted outside Nigeria
([issue #438](https://github.com/Lafiya-xyz/Lafiya-contract/issues/438)). Every
*jurisdiction* has its own health-worker licensing bodies, data-protection law, languages,
region hierarchy, and governance institutions. Today's design assumes one *admin* quorum
([ADR-0003](0003-single-admin-initial-model.md), [ADR-0007](0007-unscoped-multisig-authorization.md))
over one national *allowlist*. `attestation-registry` consults exactly one
`attester-registry` (`get_attester_registry` / `set_attester_registry`).

The naive way to scale is one global allowlist run by the Lafiya team. That puts every
country's trust in a single foreign operator, which health ministries are unlikely to
accept, and it mixes regulatory regimes in a single piece of shared state.

## Decision

**B now, C later.**

1. **Now: one deployment per jurisdiction (Topology B).** Each jurisdiction gets its own
   `attester-registry` and `attestation-registry` pair, administered by its own
   `multisig-account` whose signers belong to the national body (for example the national
   CHW programme or the ministry). The contracts don't change. Deployments are described by
   per-jurisdiction config profiles (`config/networks.toml` gains a `jurisdiction` key for
   each profile).
2. **Cross-border verification uses a trust list.** Verifiers such as `lafiya-verifier` or a
   responder app ship a signed, versioned *trust list* mapping jurisdiction codes (ISO
   3166-1 alpha-2) to trusted `attestation-registry` contract IDs and network passphrases.
   Each jurisdiction publishes its own entry through SEP-1 (`stellar.toml`) on a domain it
   controls, and the trust list pins the domain and the contract ID. If a Nigerian patient
   is treated in Ghana, the Ghanaian responder's app resolves `NG` in its trust list and
   checks the attestation against the Nigerian registry. It never needs write access or a
   Ghanaian attester.
3. **Later: federation (Topology C).** Once three or more jurisdictions are live, or when
   one attestation registry has to accept attesters from several jurisdictions (for
   example cross-border NGOs), add a root *federation* contract. It lists recognized
   national attester registries, and each entry is added or removed by a quorum of the
   member jurisdictions' admins. `attestation-registry` is then extended to accept an
   attester that is allowlisted in *any* recognized registry. The on-chain federation
   replaces the off-chain trust list as the source of truth, and B deployments join it
   without redeploying.
4. **Data protection is decided per jurisdiction, before deployment.** The on-chain footprint
   stays hash-only ([ADR-0001](0001-hash-only-on-chain-footprint.md)). Attester addresses
   are pseudonymous, but they are linkable to licensed workers through off-chain registrars.
   Before a jurisdiction is onboarded, its data-protection authority or counsel must confirm
   in writing that pseudonymous health-worker identifiers may be published on a public
   ledger. If they may not, that jurisdiction does not deploy. We don't weaken the design
   for everyone to fit one legal regime.
5. **Governance handover.** The Lafiya team may bootstrap a jurisdiction's deployment. It
   hands over control by pointing `propose_admin` at the jurisdiction's multisig and waiting
   for the multisig to call `accept_admin`. The two-step transfer is model-checked in
   `verification/tla/Governance.tla`. After handover, the Lafiya team holds no signer seat
   unless the jurisdiction asks for one. Every handover is recorded in the deployment
   ledger.

## Federation trust anchors

Decision 2 describes the trust list as an off-chain artefact. This section pins down
its format, how a verifier discovers it, how a verifier uses it, and what can go
wrong. It applies to Topology B (one deployment per jurisdiction) and is the
contract that Topology C's on-chain federation root must remain compatible with.

### Trust-anchor document format

Each jurisdiction publishes exactly one *trust-anchor document* per sequence number.
It is a JSON object with these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | Document schema version. Currently `1`. |
| `jurisdiction` | string | ISO 3166-1 alpha-2 code of the publishing deployment. |
| `network_passphrase` | string | Stellar network passphrase the registries live on. |
| `attestation_registry` | string | Contract ID of the jurisdiction's `attestation-registry`. |
| `attester_registry` | string | Contract ID of the jurisdiction's `attester-registry`. |
| `governance_signers` | array of string | Ed25519 public keys (G...) of the jurisdiction's governance multisig, sorted lexicographically. |
| `governance_threshold` | integer | Number of signatures required on this document. |
| `sequence` | integer | Monotonically increasing document sequence number. |
| `not_before` | string | RFC 3339 UTC timestamp; the document is invalid before this instant. |
| `not_after` | string | RFC 3339 UTC timestamp; the document is invalid after this instant. |
| `revoked` | boolean | When `true`, the jurisdiction withdraws its anchor; see failure modes. |
| `signatures` | array of object | `{ "signer": "G...", "signature": "<base64 ed25519>" }`, at least `governance_threshold` entries. |

The **canonical document** is the UTF-8 encoding of the JSON object with all fields
except `signatures`, keys sorted lexicographically, no insignificant whitespace, and
no trailing newline. Each signature is an Ed25519 signature over the SHA-256 digest of
the canonical document, produced by a key listed in `governance_signers`. A document
is *valid* only if at least `governance_threshold` distinct listed signers signed the
canonical document, `not_before <= now <= not_after`, and `sequence` is greater than
any sequence the verifier has already accepted for that jurisdiction.

### Key discovery

The verifier learns a jurisdiction's governance keys out of band, once, and pins
them. The pin is the pair (jurisdiction code, governance signer set) recorded in the
verifier's configuration or in a trust list it already ships. The document itself is
discovered through SEP-1: the jurisdiction's `stellar.toml` on a domain it controls
carries a `[DOCUMENTATION]`-style entry under a `LAFIYA_FEDERATION` section with
`TRUST_ANCHOR_URL` (HTTPS URL of the document) and `TRUST_ANCHOR_SHA256` (hex digest
of the document bytes). The verifier fetches the URL, checks the digest against the
pinned value, then verifies the signatures against the pinned governance keys. The
`stellar.toml` domain is pinned too, so a compromised DNS or hosting provider cannot
swap in a different document without also breaking the digest.

### Verifier algorithm for foreign attestations

Given an attestation issued by a foreign `attestation-registry`:

1. **Resolve jurisdiction.** Map the attestation's registry contract ID to a
   jurisdiction code using the verifier's pinned trust list. If no entry matches,
   fail with `unknown_jurisdiction`.
2. **Fetch and verify the trust-anchor document.** Fetch the document via the pinned
   `TRUST_ANCHOR_URL`, check `TRUST_ANCHOR_SHA256`, verify the canonical-document
   signatures against the pinned `governance_signers` and `governance_threshold`,
   check the validity window, and check `sequence` against the last accepted
   sequence. Any failure is a hard failure (see below); the verifier does not fall
   back to an older document.
3. **Check the foreign attestation and attester status.** Read the attestation from
   the foreign `attestation-registry` and the attester's status from the foreign
   `attester-registry`, both named in the document. Confirm the attestation is not
   revoked and the attester is currently allowlisted. Reads are read-only; the
   verifier never writes to a foreign deployment.
4. **Apply local policy.** The local deployment decides what a valid foreign
   attestation is worth: which jurisdictions it accepts, which attestation types it
   honours, and whether a foreign attestation alone is sufficient or must be paired
   with a local one. Policy is local configuration, not part of the document.

### Failure modes

| Failure | Cause | Verifier behaviour |
| --- | --- | --- |
| `unknown_jurisdiction` | Registry contract ID not in the pinned trust list. | Reject. Do not guess from the contract ID. |
| `anchor_unreachable` | `TRUST_ANCHOR_URL` fetch fails or times out. | Reject; retry with backoff. Never use a cached document past `not_after`. |
| `anchor_hash_mismatch` | Fetched bytes do not match `TRUST_ANCHOR_SHA256`. | Reject and alert; treat as possible tampering. |
| `anchor_signature_invalid` | Fewer than `governance_threshold` valid signatures from pinned signers. | Reject. |
| `anchor_expired` | `now` outside `[not_before, not_after]`. | Reject. |
| `anchor_stale` | `sequence` not greater than the last accepted sequence. | Reject; treat as replay. |
| `anchor_revoked` | Document has `revoked: true`. | Reject all attestations from that jurisdiction until a newer, non-revoked document is accepted. |
| `registry_unreachable` | Foreign registry read fails. | Reject; retry with backoff. |
| `attester_not_allowlisted` | Attester suspended or removed in the foreign registry. | Reject. |
| `attestation_revoked` | Attestation revoked in the foreign registry. | Reject. |

### Threat analysis: rogue-deployment injection

The main risk is an attacker standing up a deployment that claims to be, or to
speak for, a jurisdiction the verifier trusts.

- **Self-signed anchors are not trust.** An attacker can publish a perfectly
  well-formed document signed by keys they control. It is only trusted if those keys
  are in the verifier's *pinned* `governance_signers` for that jurisdiction. A
  document whose signers are not pinned fails `anchor_signature_invalid`, even if it
  is internally consistent. The verifier never learns governance keys from the
  document it is verifying.
- **Look-alike jurisdictions.** An attacker registering `NG`-like contract IDs or a
  similar domain gains nothing: the contract ID must match the pinned trust-list
  entry, and the document must come from the pinned domain and match the pinned
  digest.
- **Replay of old documents.** An attacker who captured an old, validly signed
  document could replay it to resurrect a revoked anchor or an old registry ID. The
  monotonic `sequence` check (`anchor_stale`) and the `not_after` window
  (`anchor_expired`) prevent this. Verifiers persist the highest accepted sequence
  per jurisdiction.
- **Key compromise and rotation.** If a jurisdiction's governance key is
  compromised, it publishes a new document with a higher `sequence`, a new
  `governance_signers` set, and signatures meeting the *old* threshold (so the
  rotation is itself authorized). Verifiers accept the new signer set only through
  such a document; a document that changes `governance_signers` without a valid
  signature from the previously pinned set is rejected. If the old keys are lost
  entirely, recovery requires an out-of-band trust-list update, which is why the
  trust list is versioned and signed.
- **Revocation propagation.** A jurisdiction withdraws trust by publishing a
  document with `revoked: true` and a higher `sequence`. Verifiers stop accepting
  that jurisdiction's attestations as soon as they see it. Propagation is bounded by
  how often verifiers refresh; the `not_after` window bounds how long a stale
  document can be used.

### Backward compatibility

Single-deployment verifiers are unaffected. They do not read the federation
section, do not fetch trust-anchor documents, and keep using their existing trust
anchors (the single `attestation-registry` / `attester-registry` pair they were
configured with). The federation fields are additive: a verifier that ignores them
behaves exactly as before, and a deployment that never publishes a trust-anchor
document remains verifiable by its own jurisdiction's verifiers. Mixed fleets are
supported: a verifier may enable federation for some jurisdictions and not others.

## Alternatives considered

### A. One global deployment with country-scoped registrars

A single allowlist in which each country's registrar may only manage attesters tagged with
its own region. This is the simplest to verify, since there is one contract ID. It still
needs a global admin that can upgrade the contract or override any registrar, which is the
centralization ministries would object to. Region scoping would also have to be added to
every write path, and every country's data would sit under one legal regime. Rejected.

### C. Federation from day one

This is the most expressive option, but it needs new contract code (the federation root
and multi-registry lookups in `attestation-registry`) and a cross-jurisdiction governance
process before a second country even exists. Deferred until the triggers in Decision 3
are met.

## Consequences

### Positive

- Each jurisdiction owns its allowlist and its upgrade authority, so no foreign party can
  add, suspend, or remove its health workers.
- The contracts don't change. Onboarding a country is a deployment and configuration task.
- Cross-border verification works read-only, through a trust list verifiers already have to
  pin.
- There is a clear path to C: registries deployed under B become federation members as they
  are.

### Trade-offs and risks

- Until C, the trust list is an off-chain artefact. Distributing it, signing it, and
  revoking entries is operational work, and a stale list can trust a compromised registry.
- There are N deployments to monitor, upgrade, and fund. Each one is upgraded independently,
  so versions can drift apart (the release manifest from
  [ADR-0010](0010-release-manifest-and-compatibility.md) tracks this).
- An attester licensed in two countries needs a separate allowlist entry in each registry.
- When the legal review is negative, the country is excluded rather than accommodated.

## Follow-up

To be filed as separate issues once this ADR is accepted:

- **Tooling:** per-jurisdiction config profiles (`jurisdiction` key in
  `config/networks.toml`, `lafiya-cli --jurisdiction <code>`).
- **Tooling:** implement the trust-anchor document format and canonicalization above,
  plus a verification helper in the verifier SDK.
- **Sandbox:** a read-only cross-registry check against a mock second deployment, run as
  a two-deployment sandbox scenario.
- **Contracts:** a design spike for the federation root contract, including its governance
  quorum, extended in the TLA+ models.
- **Contracts:** let `attestation-registry` consult several recognized attester registries
  (needed for C).
- **Docs:** a jurisdiction-onboarding runbook covering the legal review checklist, bootstrap
  deployment, admin handover, and the ledger entry.

## References

- [Issue #438](https://github.com/Lafiya-xyz/Lafiya-contract/issues/438)
- [ADR-0001](0001-hash-only-on-chain-footprint.md), [ADR-0003](0003-single-admin-initial-model.md),
  [ADR-0007](0007-unscoped-multisig-authorization.md), [ADR-0010](0010-release-manifest-and-compatibility.md)
- [SEP-1: stellar.toml](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md)
- `verification/tla/Governance.tla` (admin handover model)
