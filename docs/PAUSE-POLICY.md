# Pause Policy Matrix

**Principle:** Pause stops the spread of harm but never blocks remediation.

When a contract is paused, core write operations are blocked to prevent new attestations or allowlist changes during an incident. However, revocation/remediation operations and administrative actions are allowed to enable rapid incident response.

## attester-registry

| Operation | Status | Reason |
|-----------|--------|--------|
| `add_attester` | **Blocked** | Core write; prevents new fraudulent attesters during incident |
| `revoke_attester` | **Allowed** | Remediation; must remain available to remove compromised attesters |
| `is_attester` | **Allowed** | Read-only; essential for authorization checks |
| `is_paused` | **Allowed** | Read-only; needed for cross-contract pause coordination (ADR-0021) |
| `set_max_attesters` | **Allowed** | Administrative; emergency configuration changes are critical |
| `pause` | **Allowed** | Remediation; must work even when already paused |
| `unpause` | **Allowed** | Remediation; essential for recovering from incident |
| `propose_admin` | **Allowed** | Administrative; may be needed for incident response chain-of-command |
| `accept_admin` | **Allowed** | Administrative; completes remediation authorization flow |
| `upgrade` | **Allowed** | Administrative; emergency hotfix deployment may be required while paused |

## attestation-registry

| Operation | Status | Reason |
|-----------|--------|--------|
| `attest` | **Blocked** | Core write; primary target of pause during attestation incidents |
| `revoke_attestation` | **Allowed** | Remediation; must remove fraudulent attestations immediately |
| `set_attester_registry` | **Allowed** | Administrative; switching to a safe allowlist may be needed in incident |
| `is_paused` | **Allowed** | Read-only; monitoring and operational queries |
| `get_attestation` | **Allowed** | Read-only; emergency patient data access depends on this |
| `pause` | **Allowed** | Remediation; enables rapid containment |
| `unpause` | **Allowed** | Remediation; essential for recovery |
| `propose_admin` | **Allowed** | Administrative; emergency authorization changes |
| `accept_admin` | **Allowed** | Administrative; completes emergency transfers |
| `upgrade` | **Allowed** | Administrative; critical for deploying incident fixes |

## multisig-account

No pause mechanism; all operations are always available. This ensures that treasury and authorization infrastructure remain operational during incidents.

## Implementation

Each contract will include a table-driven test that:
1. Iterates over all public mutating entry points
2. Pauses the contract
3. Invokes each entry point with valid inputs
4. Asserts the documented outcome (blocked vs. allowed)

The test must **fail** if a new entry point is added without a corresponding matrix entry. This can be enforced by comparing against:
- The contract specification's function list (via `scripts/conformance/extract_interface.py`)
- Or a const list in the test that must be manually updated

Blocked operations should revert with `PausedError`; allowed operations should succeed.

## Watchdog auto-response proposals (ADR-0012, #533)

The watchdog (`crates/lafiya-watchdog`) never pauses anything itself. On a critical
alert it builds an **unsigned** pause proposal (transaction + auth entries) and
attaches it to the alert for guardians to review and sign through the multisig
tooling. Keys never live in the watchdog.

### Which paths a proposal may target

A proposed pause is scoped to the contract(s) named by the triggering rule. For each
target contract, the watchdog consults this matrix:

- **Included:** only operations marked **Blocked** in the matrix above (e.g.
  `add_attester`, `attest`). These are the paths the pause is meant to stop.
- **Excluded:** every operation marked **Allowed** — remediation (`revoke_attester`,
  `revoke_attestation`, `pause`, `unpause`), read-only (`is_attester`, `is_paused`,
  `get_attestation`), and administrative (`set_max_attesters`,
  `set_attester_registry`, `propose_admin`, `accept_admin`, `upgrade`) paths.
  A proposal that would block any of these is rejected before emission.
- **No pause mechanism:** `multisig-account` is never a pause target; proposals
  referencing it are rejected.

### Validation before a proposal is emitted

The proposal builder refuses to emit a proposal unless all of the following hold:

1. **Matrix coverage:** every public mutating entry point of each target contract has
   a matrix entry (same invariant enforced by the table-driven test above). An
   unlisted entry point aborts proposal generation rather than defaulting to blocked.
2. **Policy conformance:** the set of blocked paths implied by the proposal equals the
   matrix's **Blocked** set for that contract; any divergence (extra or missing paths)
   aborts emission.
3. **Authenticity:** the alert carries a valid guardian signature; unsigned or
   unverifiable alerts produce no proposal.
4. **Replay/rate limits:** alerts are deduplicated by nonce and bounded by the
   watchdog rate limit, so a spoofed or replayed alert cannot induce a proposal storm.

See `docs/adr/0012-coordinated-emergency-stop.md` for the spoofing and replay threat
analysis and the end-to-end sandbox demo.
