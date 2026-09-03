# ADR-0009: Stellar asset, treasury, and custody model for the USDC incentive layer

- **Status:** Proposed
- **Date:** 2026-08-18
- **Deciders:** Lafiya contract maintainers

## Context

The roadmap's **M2 — Incentives** milestone calls for a USDC-on-Stellar payout to a CHW
per verified registration, and the README lists "transparent funding" — grant and donor
funds flowing on-chain into the CHW incentive pool — as a stated goal. No payment
contract exists in this repository yet.

ADR-0007 establishes `multisig-account` as the pre-alpha administrator of
`attester-registry` and `attestation-registry`, and is explicit that this account is
**unscoped**: a valid signer quorum can authorize any invocation the signed payload
represents, including arbitrary asset transfers. ADR-0007 and the README's "Recommended
admin setup" section both instruct operators not to use that multisig as a treasury and
to keep only a small, bounded XLM fee reserve on it. That instruction is an operational
control, not an on-chain one — nothing in the contracts prevents an operator from
ignoring it, and it says nothing about what a treasury account *should* look like once
one is needed.

[Issue #136](https://github.com/Lafiya-xyz/Lafiya-contract/issues/136) asks which Stellar
asset, custody, treasury, and authorization model the incentive layer should use so that
registry administration authority and donor/incentive funds never share a trust boundary,
before any payout contract is implemented. This ADR is that decision. It does not
implement a payout contract; it specifies the custody architecture a follow-up
implementation must satisfy, per the "Recommended follow-up contract design" and
"Follow-up" sections below.

## Decision

### 1. Treasury and registry-admin are separate trust boundaries

The incentive layer must use a Stellar account/contract distinct from the
`multisig-account` deployment(s) that administer `attester-registry` and
`attestation-registry`. Concretely, a deployment needs at minimum three security
principals, each with its own signer set:

| Principal | Holds | Signs for |
| --- | --- | --- |
| Registry-admin multisig (ADR-0007) | Bounded XLM fee reserve only | `initialize`, attester add/remove/suspend, pause/unpause, contract upgrade on the two registries |
| Treasury multisig | Bulk USDC funds (grants/donations at rest) | Funding the payout contract; emergency sweep/recovery |
| Payout contract (this ADR's follow-up) | A bounded, periodically topped-up working balance | Individual CHW payouts, within on-chain rules |

No signer key may be shared across the registry-admin and treasury signer sets. This is
the same rule ADR-0007 already states for registry-admin signers versus "treasury,
personal, validator, or unrelated application authority" — this ADR just names the
treasury side of that boundary explicitly and gives it its own account instead of leaving
it undefined.

### 2. Asset identity and issuer

USDC on Stellar is a **classic Stellar asset** (code `USDC`) issued by Circle's issuer
account, published and verifiable via that issuer's `stellar.toml` (SEP-1). The issuer
account differs by network:

- **Mainnet:** Circle's public USDC issuer, verified against Circle's published
  `stellar.toml`, never hardcoded from memory into contract or deployment code without
  re-verifying against the current SEP-1 file at deployment time — issuer particulars
  are exactly the kind of value that must come from a live, dated check, not a stored
  string. The Stellar network passphrase is `Public Global Stellar Network ; September
  2015`.
- **Testnet:** Circle also publishes a testnet USDC issuer with a testnet `stellar.toml`.
  Prefer that over a self-issued test asset (a throwaway issuer account trustlined as a
  fake "USDC") specifically so the testnet rehearsal below exercises the real Stellar
  Asset Contract (SAC) wrapper and trustline behavior USDC actually has, not a
  differently-configured stand-in. The network passphrase is `Test SDF Network ; September
  2015`.

Every account or contract that is meant to hold or move USDC needs an explicit trustline
to that issuer's `USDC` asset before it can receive it (classic Stellar asset semantics —
see "Token allowance/transfer semantics" below); a contract calling into the asset via its
Soroban Asset Contract wrapper does not need a trustline of its own, since SAC balances
are tracked in contract storage, not as classic trustlines.

**Deployment configuration must record, per network:** network passphrase, the verified
USDC issuer account ID, the SAC contract ID derived from that issuer/asset pair, the
treasury multisig address, threshold, and signer public keys, the payout contract
address, and the fee-reserve ceiling for the registry-admin multisig (per ADR-0007). This
extends the deployment-record requirement ADR-0007 already places on the registry-admin
multisig to the treasury and payout principals.

### 3. Custody architecture: bounded payout contract funded from a treasury multisig

Recommended shape, from bulk funds to CHW wallet:

```text
Donor / grant funds
        |
        v
Treasury multisig  ---- periodic, multisig-approved top-up ---->  Payout contract
(bulk custody,                                                    (bounded working
 emergency sweep)                                                  balance, scoped
                                                                     payout() calls)
                                                                          |
                                                                          v
                                                                   CHW wallet address
                                                                   (validated against
                                                                    attester-registry)
```

The treasury multisig is the account of record for bulk funds and is the only principal
that can authorize a top-up transfer into the payout contract or an emergency sweep back
out. The payout contract holds only a working balance — sized to near-term payout volume,
not the full treasury — and exposes a narrow, single-purpose `payout` entry point rather
than general transfer authority. This bounds the loss from a compromised or buggy payout
contract to the current tranche, and bounds the loss from a compromised treasury signer
key to whatever a quorum can move in one transaction, without ever putting bulk custody
and per-payout logic behind the same authorization surface.

### 4. Comparison of payout custody models

| Model | Description | Blast radius of compromise | Operational cost | Verdict |
| --- | --- | --- | --- | --- |
| **A. Direct multisig custody** | Treasury multisig itself signs every individual SEP-41 `transfer` to a CHW address. No payout contract. | Any signer collusion/compromise can move the full treasury in one transaction; same shape of risk ADR-0007 already flags for admin ops. | Lowest engineering cost; highest per-payout signer burden — does not scale to per-registration micro-payments requiring N live signatures each. | Acceptable for early testnet rehearsal only; not the production target. |
| **B. Bounded payout contract with tranche top-ups (chosen)** | Treasury multisig tops up a payout contract's own SAC balance in discrete tranches; the contract's `payout` function enforces per-call and per-window caps and validates the recipient. | Compromise of the payout contract or its operator key is capped at the outstanding tranche. Compromise of the treasury multisig still requires quorum. | Moderate: one contract to build, test, and audit; top-ups are periodic multisig transactions, not per-payout. | **Recommended production model.** |
| **C. Allowance-based (SAC `approve`/`transfer_from`)** | Treasury multisig grants the payout contract a Stellar Asset Contract allowance; the contract calls `transfer_from` per payout instead of holding its own balance. | A compromised payout contract can drain up to the full outstanding allowance in one shot — the allowance amount, not a bounded tranche, is the blast radius, and allowances are easy to under-monitor. | Similar contract complexity to B, but removes the need for periodic top-up transactions. | Rejected as primary model; see "Alternatives considered." |
| **D. Custodial off-chain treasury** | Funds held with a regulated custodian/exchange off-chain; payouts triggered by an off-chain process and only recorded on-chain after the fact. | Centralizes trust in the custodian and the triggering process; on-chain records become an audit trail rather than the source of truth. | Lowest on-chain engineering cost; highest institutional/compliance overhead; opaque to on-chain observers in real time. | Rejected — contradicts the "transparent funding" / "every dollar maps to a countable number of verified cards" goal in the README. |

### 5. Multisig and role separation

Within the treasury boundary, separate the *day-to-day funding* role from the
*emergency/recovery* role, mirroring hot/cold custody practice rather than using one
signer set for both:

- **Funding operators** — a modest threshold (e.g. 2-of-3) authorized to approve routine,
  bounded top-up transfers from the treasury multisig into the payout contract.
- **Recovery signers** — a higher threshold, disjoint or partially disjoint signer set
  (e.g. 3-of-5) required for pausing the payout contract's `payout` entry point, sweeping
  the payout contract's balance back to the treasury, or rotating the treasury multisig's
  own signer set.

Neither role's signer set may overlap with the registry-admin multisig's signers (Section
1). This is a direct extension of ADR-0007's existing rule that a dedicated signer set
must not be reused across unrelated authority.

### 6. Token allowance/transfer semantics

USDC on Stellar is a classic asset (`code:issuer`), reachable from Soroban contracts
through its **Stellar Asset Contract (SAC)** wrapper, which implements the SEP-41 token
interface (`balance`, `transfer`, `approve`, `transfer_from`, ...). Two consequences drive
the design above:

- **Classic-asset accounts need a trustline.** The treasury multisig account must
  establish (and, if it ever needs to hold a new asset, remove) a trustline to the USDC
  issuer before it can hold a balance; this is ordinary classic-Stellar behavior, not
  something Soroban changes.
- **`transfer` requires `require_auth` from the source, not a delegated allowance.** A
  contract calling `transfer` as itself (source == its own SAC balance) needs no
  allowance from anyone; a contract calling `transfer_from` on someone else's balance
  needs that principal's `approve`. Model B (Section 4) uses the first shape
  deliberately: the payout contract holds its own bounded SAC balance and calls
  `transfer` as itself, so there is no standing allowance for a compromised contract to
  exploit beyond its current balance — this is precisely why Model C's allowance
  approach was rejected.

### 7. Emergency pause and recovery

The payout contract must expose:

- A `pause()` / `unpause()` pair, authorized by the recovery signer set (Section 5), that
  blocks `payout` calls without affecting the contract's stored balance or admin address
  — mirroring the `Paused`/`Unpaused` pattern already used in `attester-registry`.
- A `sweep(destination: Address)` function, authorized by the recovery signer set, that
  transfers the contract's full current SAC balance back to the treasury multisig. This
  is the recovery path if the payout contract's logic or operator key is suspected
  compromised: pause first, then sweep, then investigate before resuming.
- An `admin`-swap function following the same `Address`-based account-abstraction pattern
  ADR-0003 already uses for the registries, so the treasury multisig (or a future signer
  set) can be rotated without redeploying the payout contract.

### 8. Testnet-to-mainnet migration and rehearsal

Because the issuer account, SAC contract ID, and network passphrase all differ between
testnet and mainnet (Section 2), no address or ID from either network may be hardcoded
into contract logic; they belong only in the per-network deployment record. Before a
mainnet deployment is proposed, a testnet rehearsal must exercise the full lifecycle
end to end:

1. Fund the treasury multisig with testnet USDC (via Circle's testnet issuer, per Section
   2) and confirm the trustline.
2. Execute a multisig-approved top-up from the treasury into the payout contract.
3. Execute a batch of `payout` calls tied to real `attestation-registry` records, and
   confirm on-chain events (Section 9) match the expected recipients and amounts.
4. Exercise `pause()` and confirm `payout` calls are rejected while paused.
5. Exercise `sweep()` and confirm the full working balance returns to the treasury.
6. Exercise a treasury signer-set rotation and a payout-contract admin rotation.

Only after all six steps succeed on testnet, with the resulting deployment record filled
in per Section 2, should a mainnet deployment be proposed.

### 9. Accounting and reconciliation

The payout contract must publish an event on every state-changing call — `PayoutSent
{chw, amount, registration_id}`, `ToppedUp {amount, by}`, `Paused {by}`, `Unpaused {by}`,
`SweptOut {amount, destination}` — following the event-publishing convention already used
by `attester-registry` and `attestation-registry` (e.g. `AttesterAdded`,
`AttestationRecorded`). `registration_id` should be the same identifier
`attestation-registry` or `lafiya-web` uses for the underlying verified registration, so
off-chain reconciliation can join payout events to registration records and to Stellar
Horizon/RPC ledger data without a separate off-chain ledger of intent.

## Alternatives considered

### Reuse the registry-admin multisig as the treasury

Rejected outright — this is the exact configuration ADR-0007 already warns against ("do
not use the multisig address as a treasury or payment account"). Mixing registry
administration authority with fund custody means a single compromised or colluding
quorum could both rewrite the attester allowlist and drain donor funds, and makes it
impossible to reason about either risk independently.

### Custodial off-chain treasury (Model D)

Rejected. See Section 4 — it centralizes trust in an off-chain custodian and process,
contradicting the transparent, on-chain-verifiable funding goal stated in the README's
"Transparent funding" feature and M2 roadmap entry.

### Direct multisig custody with no payout contract (Model A)

Kept as an acceptable *interim/testnet-only* posture given its low engineering cost, but
rejected as the production target: it does not narrow authorization below "any transfer"
any more than the unscoped registry-admin multisig does (the exact problem ADR-0007
already accepts as a residual risk for registry administration), and it does not scale to
CHW payout volume, which needs per-registration payouts rather than a live quorum for
each one.

### Allowance-based payout contract (Model C)

Rejected as the primary model — see Section 4 and Section 6. A standing SAC allowance
sized for operational convenience is a larger and less visible blast radius than a
tranche-bounded balance the contract actually holds; it remains an option worth revisiting
only if tranche top-up transaction volume/fees become a real operational burden.

### On-chain `auth_contexts` scoping of a single multisig (extend ADR-0007 instead of adding a payout contract)

Considered giving `multisig-account` itself a scoping policy (allowlisting the SAC
contract and a `transfer`-with-cap pattern in `auth_contexts`) instead of introducing a
separate payout contract. Rejected for now: ADR-0007 already defers this general
capability pending an unspecified policy lifecycle (initialization, updates, recovery,
upgrade interaction), and building it generically is a larger and slower-moving problem
than a purpose-built payout contract with a fixed, auditable interface. A future
generic scoping policy on `multisig-account`, if built, would not need this ADR's payout
contract to disappear — the payout contract could simply become the thing the scoped
policy allowlists.

## Consequences

### Positive

- Registry administration authority (ADR-0007) and incentive-fund custody are provably
  separate trust boundaries with disjoint signer sets, closing the gap ADR-0007 could
  only address operationally ("do not use as a treasury") rather than architecturally.
- Compromise of the payout contract or its funding-operator signers is bounded to the
  current working-balance tranche, not the full treasury.
- The architecture follows conventions already established in this repository: Soroban
  `Address`-based account abstraction (ADR-0003), event-per-state-change (attester/
  attestation registries), and an explicit, versioned decision record for an
  administrative trust model (ADR-0007, ADR-0008).
- A concrete testnet rehearsal plan exists before any mainnet deployment is proposed.

### Trade-offs and risks

- Three security principals (registry-admin multisig, treasury multisig, payout
  contract) are more to deploy, key-manage, and monitor than one combined account.
- Tranche top-ups require periodic funding-operator transactions rather than a
  one-time allowance; if tranche sizing or cadence is wrong, payouts can stall waiting
  on a top-up, or idle balance can sit larger than necessary.
- This ADR does not itself resolve `auth_contexts` scoping for `multisig-account`
  (ADR-0007's deferred item) — the payout contract narrows authorization for *incentive
  payouts* specifically, not for registry administration generally.
- None of this has been implemented, audited, or deployed; this ADR specifies the target
  architecture and rehearsal bar, not working code.

## Follow-up

- Implement a `payout` (incentive-treasury) contract crate under `contracts/` per Section
  3, 6, and 7 (bounded balance, `payout`, `pause`/`unpause`, `sweep`, admin rotation,
  events per Section 9), with unit tests covering paused-state rejection, over-cap
  rejection, and unauthorized-caller rejection, following this repository's existing
  contract test conventions.
- Define and record the treasury multisig's funding-operator and recovery signer sets
  and thresholds (Section 5) in the deployment runbook, alongside the registry-admin
  signer-set record ADR-0007 already requires.
- Verify and record the current Circle USDC issuer `stellar.toml` (SEP-1) for both
  testnet and mainnet at deployment time, rather than trusting a previously recorded
  issuer ID indefinitely.
- Write and execute the testnet rehearsal script covering all six steps in Section 8,
  and attach its results to the deployment record before a mainnet proposal.
- Coordinate with `lafiya-web` on a shared `registration_id` format so payout events
  (Section 9) can be joined to registration records for reconciliation.
- Decide tranche sizing and top-up cadence once real CHW payout volume estimates exist
  from the M3 pilot roadmap milestone.

## References

- [Issue #136](https://github.com/Lafiya-xyz/Lafiya-contract/issues/136)
- [ADR-0003: Use a single admin address for the pre-alpha contracts](0003-single-admin-initial-model.md)
- [ADR-0007: Keep multisig authorization unscoped during pre-alpha](0007-unscoped-multisig-authorization.md)
- [`contracts/multisig-account/src/lib.rs`](../../contracts/multisig-account/src/lib.rs)
- [`contracts/attester-registry/src/lib.rs`](../../contracts/attester-registry/src/lib.rs)
- [README — Recommended admin setup, Roadmap](../../README.md)
- [SEP-1 — `stellar.toml`](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md)
- [SEP-41 — Token interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [Stellar Asset Contract documentation](https://developers.stellar.org/docs/tokens/stellar-asset-contract)
