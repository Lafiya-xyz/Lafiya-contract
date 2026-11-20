# ADR 0012: Multisig Account Upgradeability

- **Status:** Accepted
- **Date:** 2025-01-01
- **Deciders:** Multisig maintainers, Security reviewers
- **Related:** Issue #709, `multisig-account`, registry `upgrade`/`migrate` paths, upgrade-authority renunciation, upgrade-compatibility harness, watchdog

## Context

`multisig-account` is the account contract that administers the registries. The registries already expose `upgrade` / `migrate` entry points and an upgrade-authority renunciation path. The account itself has **no** `upgrade` entry point.

This asymmetry creates a real operational risk. If a defect exists in `__check_auth` — the very code path that decides whether an operation is authorized — then the only current remedy is to deploy a brand-new account contract and migrate admin rights on every registry. That migration must be authorized by the **old** account. If the bug is in authorization itself, the old account may be unable to authorize its own replacement, leaving the registries permanently administered by a broken account.

We must decide, explicitly and durably, whether the account is:

- **immutable by design**, with a documented and rehearsed migration procedure, or
- **upgradable under strict guards**, with a pending-upgrade state, a mandatory delay, an announcement event, and the ability for guardians to cancel.

The decision must weigh the security posture of an account whose whole purpose is to gate privileged registry operations against the operational necessity of being able to recover from an authorization defect.

### Constraints and assumptions

- The account holds the admin authority for one or more registries. Compromise of the account is equivalent to compromise of those registries.
- Guardians (a threshold of independent parties) already exist as a trust anchor for recovery-style operations.
- Any upgrade path is itself a privileged operation and must be authorized under the **strictest** policy the account supports, not the routine policy.
- The watchdog already monitors registry upgrades; it can be extended to monitor account upgrades.
- Synthetic data only in tests and rehearsals; no real secrets or payloads.

## Options Considered

### Option A — Immutable by design, plus documented migration and rehearsal

The account has no `upgrade` entry point. Recovery from a defect is handled by deploying a new account and migrating admin rights on every registry, following a documented runbook that is rehearsed end to end in the sandbox.

**Advantages**

- Smallest attack surface: there is no code path by which an attacker who compromises the account can replace its logic.
- Strong, simple invariant: the account's behavior is fixed at deploy time and auditable once.
- No new privileged state machine (pending upgrades, delays, cancellations) to get wrong.
- Aligns with the principle that the most security-critical contract should change least.

**Disadvantages**

- The migration requires the **old** account to authorize. If the defect is in `__check_auth`, this may be impossible — the exact scenario the issue calls out.
- Migration touches every registry, is multi-step, and is error-prone under incident pressure.
- A rehearsal reduces but does not eliminate the risk that the real migration fails when authorization is broken.
- No in-place fix means a known bug persists until a full migration succeeds.

### Option B — Guarded `upgrade(new_wasm)` with strictest policy, mandatory delay, and announcement event

Add an `upgrade(new_wasm)` entry point that:

1. Requires authorization under the **strictest** policy (highest threshold / all required signers), not the routine policy.
2. Does **not** apply the new code immediately. Instead it records a **pending upgrade** (new wasm hash, activation timestamp) and emits an **announcement event**.
3. Enforces a **mandatory delay** before the pending upgrade can be activated, giving guardians and operators time to observe and react.
4. Requires a separate activation step after the delay elapses.

**Advantages**

- Directly addresses the motivating failure: a bug in `__check_auth` can be fixed in place, without depending on the broken authorization path to migrate registries.
- The delay plus announcement gives an observation window; a malicious or mistaken upgrade is visible before it takes effect.
- Strictest-policy authorization raises the bar well above routine operations.
- Pending state is explicit and inspectable, and the watchdog can alert on it.

**Disadvantages**

- Introduces new privileged state and transitions (propose → delay → activate) that must be correct.
- An attacker who can satisfy the strictest policy can still eventually upgrade; the delay only buys reaction time.
- Adds complexity to the account and to the test/harness surface.
- Without a cancellation mechanism, the only defense during the delay is off-chain reaction.

### Option C — Option B plus a renunciation flag

Everything in Option B, plus a one-way **renunciation flag** that permanently disables the upgrade path. Once renounced, the account behaves as in Option A (immutable), and the pending-upgrade machinery can no longer be used.

**Advantages**

- All the recovery benefits of Option B during the account's early, higher-risk life.
- Lets the operators **converge to immutability** once the account has stabilized, giving the strongest long-term guarantee without a redeploy.
- Renunciation is a clear, auditable, one-way commitment that can be announced and verified.
- Guardians can cancel a pending upgrade, and renunciation closes the door entirely when appropriate.

**Disadvantages**

- Largest surface area of the three: pending state, delay, activation, cancellation, and renunciation.
- Renunciation is irreversible; a mistaken renunciation removes the recovery path permanently.
- More states and transitions to test and to monitor.

## Decision

We choose **Option C — guarded upgrade plus renunciation**, implemented as follows:

1. **`upgrade(new_wasm)`** requires authorization under the account's **strictest** policy. It does not apply code immediately; it records a **pending upgrade** (wasm hash and earliest activation time) and emits an **announcement event**.
2. A **mandatory delay** must elapse before activation. Activation is a separate, explicitly authorized step.
3. **Guardians can cancel** a pending upgrade at any time before activation.
4. A **renunciation** operation permanently disables the upgrade path (one-way). After renunciation, `upgrade` and activation are rejected and the account is immutable by design.
5. The account is added to the **upgrade-compatibility harness**, and the **watchdog alerts on pending account upgrades** (and on renunciation).

### Rationale

Option A alone does not satisfy the motivating requirement: it cannot recover from a defect in `__check_auth`, because recovery depends on the very authorization that may be broken. Option B fixes that but leaves the account permanently upgradable with no path to the stronger immutability guarantee. Option C gives us recovery when we need it and a credible route to immutability once the account has matured, at the cost of additional, well-scoped state that we can test and monitor. The mandatory delay, announcement event, strictest-policy authorization, and guardian cancellation together ensure that no single compromised signer or mistaken operation can silently replace the account's logic.

## Consequences

### Positive

- In-place recovery from authorization defects is possible without migrating every registry.
- Pending upgrades are observable (event + state) and cancellable by guardians.
- Operators can converge to immutability via renunciation.
- The watchdog gains a concrete signal (pending account upgrade) to alert on.

### Negative / risks

- New privileged state machine must be implemented and audited carefully.
- Renunciation is irreversible; it must be a deliberate, announced action.
- The delay window is the primary defense against a malicious upgrade; its length is a policy parameter that must be chosen and documented.

### Mitigations

- Strictest-policy authorization for both proposal and activation.
- Mandatory delay with an announcement event; guardians can cancel.
- End-to-end sandbox tests covering propose → delay → activate, propose → cancel, and renounce → reject.
- Watchdog alerts on pending upgrades and on renunciation.
- Account added to the upgrade-compatibility harness.

## Implementation Notes

- **State:** pending upgrade record (wasm hash, earliest activation time), renounced flag.
- **Entry points:** `upgrade(new_wasm)` (propose), `activate_upgrade()` (after delay), `cancel_upgrade()` (guardians), `renounce_upgrade()` (one-way).
- **Events:** upgrade announced, upgrade activated, upgrade cancelled, upgrade renounced.
- **Authorization:** proposal and activation require the strictest policy; cancellation requires guardian threshold; renunciation requires the strictest policy.
- **Watchdog:** alert on any pending upgrade and on renunciation.

## Testing / Rehearsal

- Sandbox end-to-end: propose → observe announcement → wait delay → activate; verify new behavior.
- Sandbox end-to-end: propose → guardian cancel → verify no activation possible.
- Sandbox end-to-end: renounce → verify `upgrade`/activation rejected and account immutable.
- Upgrade-compatibility harness includes the account.
- Watchdog test: pending upgrade raises an alert.

## Alternatives Rejected

- **Option A (immutable only):** rejected as the sole strategy because it cannot recover from a `__check_auth` defect.
- **Option B (guarded upgrade without renunciation):** rejected as the final choice because it forgoes the ability to converge to immutability; retained as a subset of the chosen design.

## References

- Issue #709 — decide on and implement a guarded self-upgrade path.
- Registry `upgrade` / `migrate` and upgrade-authority renunciation paths.
- Upgrade-compatibility harness.
- Watchdog alerting.
- `SECURITY.md` — security-sensitive details belong in a private advisory.