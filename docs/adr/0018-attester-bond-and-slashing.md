# ADR-0018: Optional attester bonds with quorum-gated, appealable slashing

- **Status:** Proposed
- **Date:** 2026-10-17
- **Deciders:** Lafiya contract maintainers

> **Terminology:** shared terms (*attester*, *CHW*, …) are defined in the [glossary](../glossary.md).

## Context

Removal is the only sanction `attester-registry` has today. It is cheap to evade: a
removed attester enrolls a new key. Issue #587 asks for economic accountability, a bond
that is forfeited when an attestation is later proven fraudulent, which raises the cost
of Sybil identities and funds remediation. This ADR precedes the implementation, as the
issue requires, and must answer: where the bond lives, who can slash and how, how bond
size relates to payout, and what rotation does to a bond.

## Decision

Bonds are **optional and policy-configured**: with no `BondConfig` nothing changes. When
configured (`set_bond_config`, admin), an enrolled attester may escrow a bond in the
configured token (`post_bond`). `is_bonded` reports whether the unlocked bond meets
`min_bond`; consumers (the pool, a verifier, an operator promoting out of probation, see
[`attester-probation.md`](../specs/attester-probation.md)) may use it as a signal. This
ADR does not make a bond mandatory for attesting.

### Custody: escrow in `attester-registry`, accounted separately from the pool

The registry holds bond tokens on its own address. Bond balances are tracked per attester
(`Bond(Address)`) and never mix with `incentive-pool` treasury accounting, so pool
solvency and `withdraw` logic are unaffected.

- *Pool custody* was rejected: it couples bond solvency to the pool's pause, drain and
  payout paths and gives the pool admin and the registry's slashers overlapping power over
  the same funds.
- *A separate vault contract* is the cleanest isolation but adds a contract, a deployment
  and cross-contract auth on every slash for little gain at pre-alpha scale. It stays a
  follow-up if the registry's attack surface becomes a concern; the `Bond*` API is
  vault-shaped, so the move would not change callers.

### Release conditions

A bond leaves escrow only through (a) `withdraw_bond`, after `request_bond_withdrawal`
plus `unbond_delay` and with **zero open slash cases**, or (b) `execute_slash`, which
pays the slashed amount to `BondConfig.recipient` (a remediation fund or the pool). Removal
or self-revocation does **not** release a bond: the unbonding cooldown still applies, so
leaving the allowlist cannot be used to dodge a case that is still being assembled.
`unbond_delay` must exceed `challenge_window`, so a case proposed on the last day of the
cooldown can still finish.

### Slash authorization: quorum, then challenge window, then appeal

1. `propose_slash` (registrar or guardian) opens a case with an `evidence_hash` (a
   hash of the off-chain dossier, not health data) and **locks** the amount, so the
   attester cannot withdraw or be double-slashed past the bond.
2. `approve_slash` needs `quorum` **distinct** registrar/guardian accounts, the proposer
   counting as the first. At quorum the case becomes `Challengeable` and the window
   (`challenge_window`) starts.
3. During the window the attester may `appeal_slash`, freezing the case. The **admin**
   (a multisig in production, see ADR-0007) rules with `resolve_appeal`: upheld makes the
   case executable immediately; rejected cancels it and unlocks the bond.
4. Otherwise anyone may `execute_slash` once the window has elapsed.
5. The admin may `cancel_slash` any unexecuted case (for example on new exculpatory
   evidence).

The window is the only appeal gate: after it closes unappealed, the slash is final. At most
`MAX_OPEN_CASES = 5` cases may be open per attester, which bounds rotation work and
prevents locking-by-spam from exhausting an attester's bond with duplicate cases.

### Key rotation

`rotate_attester` moves the bond, its locks, and every open case to the successor key and
re-points `SlashCase.attester`, atomically in the rotation transaction. A rotation can
therefore neither shed a pending slash nor strand a bond on a retired key, and the
successor (not the retired key) holds the right to appeal. A bond already on the successor
address is merged.

## Economic analysis

Let the attacker enroll one identity, post bond `B`, and attest fraudulently to collect a
pool payout `p` per verified card. Let `q` be the per-card probability that a fraudulent
attestation is detected and slashed (audit sampling rate times audit accuracy). Fraud
before detection is geometric, so the expected number of paid fraudulent cards is `1/q`
and the expected gross gain is

    G = p / q, capped by the pool's per-attester limit M (max_per_attester)

The attack is unprofitable for a risk-neutral attacker when the expected loss exceeds the
gain: `B + c > min(G, M)`, where `c` is the cost of obtaining an identity and surviving
probation (KYC effort, delay). The probation rate cap `r` per window additionally bounds
gain per unit time, so detection has `r`-limited exposure to accumulate before a bond has
to stand on its own.

| p (USDC/card) | q (detect/card) | G = p/q | Bond needed (c = 0) |
| ---: | ---: | ---: | ---: |
| 1 | 5% | 20 | 20 |
| 1 | 2% | 50 | 50 |
| 2 | 2% | 100 | 100 |
| 1 | 0.5% | 200 | min(200, M) |

Sybil scale: `k` fake identities lock `k·B`, so collusion costs grow linearly with
the attack surface, while the gain of each identity stays capped by `r·p` per window until
detection. The design target is `B ≈ min(p/q, M)`; the table's values are illustrative
inputs to be replaced with measured `p` and audit rates before mainnet.

Cost to honest attesters: capital cost is `B·i` per period (`i` = opportunity cost of
capital). For `B = 50` and `i = 10%`/year that is about 0.42 USDC/month, small against
earnings, so the real barrier for a CHW is **liquidity**, not return. Programmes
can fund a bond by transferring tokens to the attester, who posts it; the contract only
accepts a bond from the attester's own account.

False-slash risk is the cost on the other side: an erroneous slash destroys an honest
attester's savings. It is bounded by the quorum of distinct accounts, the evidence hash,
the challenge window, the admin appeal, and the slash amount being capped at the unlocked
bond. If `Pr(false slash)` after appeals is `f`, the expected honest cost is `f·B` per
case; the process should be tuned so `f·B` stays well below monthly earnings.

## Alternatives considered

- **Mandatory bond at enrollment.** Strongest deterrent but excludes the lowest-capital
  CHWs and changes every existing enrollment flow. Kept optional.
- **Unilateral admin slash.** Simple but a single compromised key could confiscate every
  bond. Rejected in favor of quorum plus window plus appeal.
- **Time-lock only, no appeal.** Cheaper but offers the accused no recourse. Rejected.

## Consequences

- New contract state and an escrow role for `attester-registry`; storage keys are
  appended to `DataKey`, no migration is needed, and instance TTL handling matches existing
  entry points.
- The admin is the appeal authority, which concentrates a judicial role on the admin
  account. A dedicated arbiter role is a follow-up.
- Bonds and slashes surface as events (`BondPosted`, `SlashProposed`, `SlashApproved`,
  `SlashAppealed`, `SlashAppealResolved`, `SlashExecuted`, `SlashCancelled`); the watchdog
  alerts on `slash_executed`.
- Token behavior is trusted to be a standard Stellar asset contract; fee-on-transfer or
  rebasing tokens are out of scope.

## Follow-up

- Measure `p`, `q` and `M` and publish the chosen `min_bond`.
- Optional vault contract if custody isolation is required.
- Dedicated arbiter role separate from the admin.

## References

- Issue #587
- [ADR-0009](0009-treasury-asset-custody-model.md), [ADR-0012](0012-chw-attester-key-custody.md)
- `contracts/attester-registry/src/bond.rs`
