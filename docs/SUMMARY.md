# Summary

[Overview](overview.md)

# Concepts

- [Glossary](glossary.md)
- [Event indexing and data flow](architecture/event-indexing.md)
- [Storage versioning](architecture/storage-versioning.md)
- [Storage cost](storage-cost.md)

# Contracts

- [attester-registry](reference/contracts/attester-registry.md)
- [attestation-registry](reference/contracts/attestation-registry.md)
- [multisig-account](reference/contracts/multisig-account.md)
- [Error codes](error-codes.md)
- [Event schemas](events.md)

# Integrate

- [TypeScript bindings](typescript-bindings.md)
  - [attester-registry bindings](imported/bindings-attester-registry.md)
  - [attestation-registry bindings](imported/bindings-attestation-registry.md)
  - [Publishing](imported/publishing.md)
- [Record commitments (lafiya-commitment)](imported/lafiya-commitment.md)
- [RPC resilience (lafiya-rpc-resilience)](imported/lafiya-rpc-resilience.md)
- [Local sandbox](imported/sandbox.md)

# Operate

- [Runbook: contract upgrade](runbooks/contract-upgrade.md)
- [Runbook: RPC outage recovery](runbooks/rpc-outage-recovery.md)
- [CLI reference: lafiya-cli](reference/cli/lafiya-cli.md)
- [CLI reference: cargo xtask](reference/cli/xtask.md)
- [Network configuration](imported/config.md)
- [Scripts](imported/scripts.md)
- [Release process and deployment ledger](releasing.md)
- [Changelog](imported/changelog.md)

# Decisions

- [ADR index](adr/README.md)
  - [ADR-0001: Hash-only on-chain footprint](adr/0001-hash-only-on-chain-footprint.md)
  - [ADR-0002: contractclient boundary](adr/0002-contractclient-boundary.md)
  - [ADR-0003: Single admin initial model](adr/0003-single-admin-initial-model.md)
  - [ADR-0006: Attestation revocation semantics](adr/0006-attestation-revocation-semantics.md)
  - [ADR-0007: Unscoped multisig authorization](adr/0007-unscoped-multisig-authorization.md)
  - [ADR-0008: Record commitment canonicalization](adr/0008-record-commitment-canonicalization.md)
  - [ADR-0009: Treasury and custody model](adr/0009-treasury-asset-custody-model.md)
  - [ADR-0010: Release manifest and compatibility](adr/0010-release-manifest-and-compatibility.md)
  - [ADR-0011: RPC failover and transaction recovery](adr/0011-rpc-provider-failover-and-transaction-recovery.md)
  - [ADR-0012: Multi-jurisdiction deployment topology](adr/0012-multi-jurisdiction-deployment-topology.md)
  - [ADR template](adr/0000-template.md)
- [Spike: contract interface conformance](spikes/0137-contract-interface-conformance.md)

# Security

- [Disclosure policy](imported/security.md)
- [Formal verification (TLA+)](imported/formal-verification.md)

# Reference

- [API (rustdoc)](api.md)
- [Contributing](imported/contributing.md)
