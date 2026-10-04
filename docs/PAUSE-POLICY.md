# Pause Policy Matrix

**Principle:** Pause stops the spread of harm but never blocks remediation.

When a contract is paused, core write operations are blocked to prevent new attestations or allowlist changes during an incident. However, revocation/remediation operations and administrative actions are allowed to enable rapid incident response.

## attester-registry

| Operation | Status | Reason |
|-----------|--------|--------|
| `add_attester` | **Blocked** | Core write; prevents new fraudulent attesters during incident |
| `revoke_attester` | **Allowed** | Remediation; must remain available to remove compromised attesters |
| `is_attester` | **Allowed** | Read-only; essential for authorization checks |
| `is_paused` | **Allowed** | Read-only; needed for cross-contract pause coordination (ADR-0012) |
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
