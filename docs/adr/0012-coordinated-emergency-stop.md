# ADR-0012: Coordinated Emergency Stop across Registries

- **Status:** Proposed
- **Date:** 2026-09-26
- **Deciders:** Maintainers and relevant reviewers

## Context

The `attester-registry` and `attestation-registry` contracts have independent pause mechanisms. When paused:
- `attester-registry` blocks allowlist modifications but does not prevent `is_attester` reads or ongoing attestations by allowlisted attesters
- `attestation-registry` blocks `attest` calls but does not prevent allowlist changes in the attester registry

In incident scenarios such as "allowlist tampering" or "batch of fraudulent CHW keys added", responders naturally pause the attester registry first. However, this does nothing to stop fraudulent attestations from existing allowlisted attesters. A separate pause on the attestation registry is required to fully contain the incident. This two-step coordination across contracts (potentially behind different quorums) delays incident response and increases operational complexity.

The system needs a fail-safe mechanism where pausing either registry effectively halts all attestation activity.

## Decision

Implement **Option A: Pause Propagation** with a combined read pattern.

The `attester-registry` contract will expose a public `is_paused()` query. The `attestation-registry::attest` function will check both:
1. Its own pause state (existing behavior)
2. The attester registry's pause state (new cross-contract read)

The check will be optimized by introducing a new helper `is_attester_active(attester: Address) -> bool` that combines both the `is_attester` authorization check and the pause state check into a single operation, reducing gas costs and ensuring atomicity of the decision.

### Implementation requirements:
- `attester-registry::is_paused() -> bool`: Public query, no authorization required
- `attestation-registry::is_attester_active(attester)`: Combines `is_attester(attester)` + "both pause flags are not set"
- Update `attest` to use `is_attester_active` and refuse with `PausedError` if it returns false
- Document that operators should pause `attester-registry` first in incident runbooks; the cross-contract read ensures containment even if the second pause fails

## Alternatives considered

### Option B: Global circuit-breaker contract

A new `lafiya-guardian` contract holds a single `halted` flag queried by both registries. One transaction by the guardian role stops the entire system.

**Rejected because:**
- Adds a new contract and deployment complexity
- Introduces a third authorization/quorum path, potentially increasing time-to-decision
- The cross-contract read in Option A (one extra SOROBAN_COST_LOAD_DATA call) has minimal gas cost
- Option B does not reduce total pause targets; responders still need to be drilled on the guardian role

### Option C: Operational only

Document in the incident runbook that both registries must be paused, with a CLI `emergency-stop` command that submits both pauses atomically.

**Rejected because:**
- Relies on human coordination and CLI invocation; no technical guarantee of containment
- If the second pause fails or is forgotten, fraudulent attestations continue
- Does not improve mean-time-to-containment

## Consequences

### Positive

- Single action (pause attester-registry) is now sufficient to halt all attestation activity
- No new contract required; minimal code changes
- Gas cost of one cross-contract read is negligible compared to security benefit
- Responders see immediate effect; no coordination required

### Trade-offs and risks

- Attestation-registry now depends on attester-registry availability; a network issue affecting attester-registry could prevent attestations even if the attestation registry itself is healthy
- Off-chain systems monitoring pause state must now query two contracts to determine if attestations are accepting
- Cross-contract call introduces a small latency increase per attest (typically <5ms on live networks)

## Follow-up

1. Implement `is_paused()` in `attester-registry` and expose it in the contract spec
2. Implement `is_attester_active()` in `attestation-registry` and refactor `attest` to use it
3. Add integration test verifying that pausing attester-registry halts attest calls
4. Update incident runbooks to document "pause attester-registry to halt all attestations"
5. Measure and document the gas cost of the added cross-contract read

## References

- Issue #365: Design a coordinated emergency stop across both registries
- Related: ADR-0003 (single admin initial model) — pausing is currently an admin-only action
- Related: ADR-0007 (unscoped multisig authorization) — applies to pause authorization
