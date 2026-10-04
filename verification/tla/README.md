# TLA+ models of the governance state machines

Bounded TLC models of the admin, pause, and upgrade/migrate state machines, plus a
design check for multisig signer rotation racing a guardian recovery. CI runs them on
pushes to `main` and on PRs that touch the specs or contracts (`.github/workflows/tla.yml`).

## Running

Requires Java 11+.

```bash
verification/tla/run.sh                      # downloads the pinned tla2tools jar
verification/tla/run.sh path/to/tla2tools.jar  # or use a local copy
```

`run.sh` pins `tla2tools.jar` **v1.8.0** by SHA-256 and checks every `*.cfg` in this
directory:

| Config | Expected |
| --- | --- |
| `<Spec>.cfg` | passes: every invariant and property holds |
| `<Spec>.<name>-counterexample.cfg` | **fails** with an invariant violation; it records an accepted finding and stops passing only if the design changes |

## Specs

### `Governance.tla`: admin lifecycle, pause, upgrade

| Action | Contract function |
| --- | --- |
| `ProposeAdmin(r, a)` | `propose_admin` (both registries) |
| `AcceptAdmin(r)` | `accept_admin` (both registries) |
| `Pause(r)` / `Unpause(r)` | `pause` / `unpause` (both registries) |
| `Upgrade(r, s)` | `upgrade` (`attester-registry`); `s` is the new build's `SCHEMA_VERSION` |
| `Migrate(r)` | `migrate` (`attester-registry`) |
| `CanAuth(a)` | `require_auth` succeeds for `a` |

`attestation-registry` has no `upgrade`/`migrate` yet, so it is excluded from `Upgradable`.
The contracts have no proposal `cancel` or expiry, so the spec doesn't model them either
(see Findings).

Properties:

| Name | Kind | Meaning |
| --- | --- | --- |
| `AdminAlwaysCanSign` | safety | exactly one admin, and it can sign: transferring to an address nobody controls never completes, because `accept_admin` needs the new admin's signature |
| `NoStaleProposal` | safety | a pending proposal was always made by the *current* admin; `accept_admin` clears it, so it can't outlive a transfer |
| `UnpauseAlwaysEnabled` | safety | pausing never prevents the admin from unpausing |
| `MigrateAlwaysEnabled` | safety | a pending migration can always be applied, including while paused |
| `StoredSchemaNotAhead` | safety | storage is never newer than the code that reads it |
| `EventuallyUnpaused` | liveness | with an honest, available admin, a paused registry is eventually unpaused |
| `EventuallyMigrated` | liveness | with an honest, available admin, an upgrade is eventually followed by a completed migration |

Bounds (`Governance.cfg`): 3 accounts (one of them unsignable, like a typo'd address), 2
registries, schema versions 1..3. That gives 1,536 distinct states in about 3 seconds.

### `MultisigRecovery.tla`: rotation vs. guardian recovery (design check)

`contracts/multisig-account` doesn't have timelocked rotation or guardian recovery yet.
This spec settles a design requirement before either is built.

| Action | Intended function |
| --- | --- |
| `ProposeRotation(t)` / `ApplyRotation` | quorum-approved threshold change, applied after a delay |
| `ProposeRecovery(S, t)` / `ApplyRecovery` | guardian replaces the signer set and threshold after a delay |
| `DropStale` | discard a proposal made against an older configuration |

Property: `NeverLockedOut`: the threshold never exceeds the number of signers whose keys
are still available.

Bounds: 4 keys (one lost), 2 recovery sets, at most 3 applied changes: 1,296 distinct
states.

## Findings

| # | Finding | Resolution |
| --- | --- | --- |
| 1 | **Rollback leaves storage ahead of code.** `upgrade` accepts any Wasm hash, so installing an older build after `migrate` leaves `SchemaVersion` greater than the code's `SCHEMA_VERSION`, and `migrate` then returns `MigrationNotRequired`. Reproduced by `Governance.rollback-counterexample.cfg`. | **Accepted**: `docs/runbooks/contract-upgrade.md` §6 tells operators to roll forward, not back, once `migrate` has run. `Governance.cfg` models that discipline (`AllowRollback = FALSE`). |
| 2 | **Stale rotation locks everyone out.** A threshold increase proposed against the old signer set applies after a guardian recovery shrank the set, so the threshold exceeds the available signers. Reproduced by `MultisigRecovery.no-nonce-counterexample.cfg`. | **Fixed in design**: every configuration change must carry the configuration nonce it was proposed against and be rejected if the nonce changed (`UseNonce = TRUE`). |
| 3 | **Proposals never expire and can't be cancelled.** A pending admin proposal stays acceptable until overwritten. `NoStaleProposal` still holds, because only the current admin can have made it. | **Accepted**: to withdraw a proposal, the admin proposes itself. Revisit if a timelock is added. |

## Extending

1. When a contract function that changes governance state is added or changed, update the
   matching action and the mapping table above in the same PR.
2. State new guarantees as named operators and list them under `INVARIANTS` or
   `PROPERTIES` in the `.cfg`.
3. When TLC finds a counterexample, either fix the design or keep it as a
   `*-counterexample.cfg` and add a row to Findings.
4. Keep the bounds small enough for `run.sh` to finish in about a minute.
