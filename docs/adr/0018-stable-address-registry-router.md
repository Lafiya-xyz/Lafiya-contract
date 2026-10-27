# ADR-0018: Stable-Address Registry Router

- **Status:** Proposed
- **Date:** 2026-10-21
- **Deciders:** Maintainers and relevant reviewers

## Context

Re-genesis ([ADR-0016](0016-registry-re-genesis-state-migration.md)) and repointing change
contract ids. Clients hardcode ids from config and break unless redeployed in lockstep.
Integrators want one address that never changes and that tells them where each contract
currently lives and which interface it speaks.

## Decision

Provide an **optional** router contract. Direct-id mode (configuring contract ids from
`stellar.toml`, the release manifest, or `config/networks.toml`) stays fully supported and
is the default; nothing in the verification path requires the router.

### Contract surface (minimal)

| Function | Auth | Behavior |
|---|---|---|
| `get_state() -> RouterState` | none | `{ seq, entries: Map<Symbol, RouterEntry> }` |
| `get(name) -> Option<RouterEntry>` | none | One entry |
| `set(name, contract_id, interface_version)` | admin | Upserts an entry, `seq += 1`, emits `router_set` |
| `remove(name)` | admin | Deletes an entry, `seq += 1`, emits `router_removed` |
| `propose_admin` / `accept_admin` | admin / new admin | Two-step transfer, as in the registries |
| `renounce()` | admin | Freezes the router permanently (see Migration) |

`RouterEntry = { contract_id: Address, interface_version: u32 }`. Entries are versioned by
`seq`, which only increases. Every change appends to the **event log** (`router_set`,
`router_removed`) with `{ name, old, new, seq }`, so indexers can replay history and detect
gaps.

### Interface-version integration

`set` calls `get_interface()` on the target contract (see
`crates/lafiya-cli/src/interface.rs`) and stores the returned `interface_version`; it rejects
a target that does not implement the view or whose `contract_kind` does not match `name`.
Clients pass `minInterfaceVersion` to the SDK resolver and refuse entries below it, so a
router update cannot silently downgrade a client.

### Client resolution (SDK)

`resolveContract` in `packages/verifier-sdk/src/router.ts` reads `get_state`, then:

- remembers the last-known state with a SHA-256 over its canonical form (a corrupt local
  cache is a miss);
- rejects any state whose `seq` is lower than the remembered one (rollback);
- falls back to the remembered state, flagged `cached: true`, when the router is unreachable;
- enforces `minInterfaceVersion`.

### Router migration

The router is itself a contract and can be re-genesised. Two modes, chosen at deployment:

1. **Mutable (default):** upgradeable under the same governance as the registries
   ([ADR-0015](0015-upgrade-authority-renunciation.md)). A replacement router is published
   out-of-band (stellar.toml, manifest) and clients re-pin its id once.
2. **Immutable:** admin calls `renounce()` and upgrade authority is renounced. The address
   and its entries are then fixed forever; to change a target the integrator must move to a
   new router. Choose this only when the entries are themselves stable.

## Threat analysis: malicious router update

| Threat | Impact | Mitigation |
|---|---|---|
| Compromised admin repoints `attestation-registry` to an attacker contract | Clients verify cards against forged attestations | The router is **not** a trust anchor for high-assurance verifiers: they keep direct-id pins (default). Admin should be the multisig ([ADR-0007](0007-unscoped-multisig-authorization.md)) behind the timelock ([TIMELOCK-DESIGN](../TIMELOCK-DESIGN.md)), so a repoint is visible before it takes effect. Clients pin `contract_kind` and `minInterfaceVersion`; `set` verifies both on-chain. |
| Rollback to an old entry | Revoked or superseded deployment becomes current again | `seq` is monotonic; the SDK rejects lower `seq` than last seen. |
| Router unreachable or censored | Clients cannot resolve | Fall back to cached state (flagged); direct-id mode remains available. |
| Silent downgrade of `interface_version` | Client talks to an incompatible ABI | `minInterfaceVersion` check; `set` reads the version from the target, never from the caller. |
| Event-log gap hides a change | Auditors miss a repoint | `seq` is carried in every event; indexers alert on gaps. |
| Hijacked router initialization | Attacker owns admin | Admin is a constructor argument (no `initialize`); deployment is recorded in the deployment ledger. |

A malicious or mistaken update therefore degrades to **unavailable** (interface check or
rollback rejection), not to **silently verified against the wrong contract**, for any client
that sets `minInterfaceVersion`, tracks `seq`, and keeps its trust anchors pinned.

## Consequences

- Integrators get one immutable address, at the cost of trusting the router's admin for
  resolution; the trust anchors remain explicit and pinnable.
- The router contract implementation is a follow-up; this ADR fixes its surface, events, and
  the SDK contract so both can be built against it. Adding the contract changes the public
  surface and requires the conformance snapshot update described in
  [stability-policy](../stability-policy.md).
