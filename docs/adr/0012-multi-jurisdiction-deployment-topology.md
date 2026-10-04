# ADR-0012: Multi-jurisdiction deployment topology

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
- **Tooling:** a trust-list format (JSON, signed, versioned) plus a verification helper in
  the verifier SDK.
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
