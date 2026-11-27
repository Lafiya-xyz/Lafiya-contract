# Formal Verification Invariants

## Tool Decision: Kani (Rust Model Checker)

**Chosen:** Kani over Certora Sunbeam
- **Reason:** Kani works directly on Rust source; no need for separate spec language; better integration with Soroban SDK
- **Trade-off:** Certora Sunbeam would verify WASM directly, but Kani's source-level analysis catches more semantic issues

## Attester Registry Invariants

### Invariant 1: Attester Existence and Non-Suspension
```
∀ a : Address,
  is_attester(a) = true  ⟹  (Attester(a) ∈ storage ∧ ¬Suspended(a))
```

**Verification:** Kani model checker over `is_attester()` and `suspend_attester()` functions
**Property:** Once suspended, is_attester must return false until reinstated
**Test:** Verify across all state mutations (add, suspend, reinstate, remove)

### Invariant 2: Attester Count Integrity
```
AttesterCount = |{a : Attester(a) ∈ storage ∧ ¬Suspended(a)}|
```

**Verification:** Kani symbolic execution over batch operations
**Property:** Count never mismatches actual allowlist size
**Test:** Verify after add_attester, remove_attester, batch_add, suspend, reinstate

### Invariant 3: Admin-Only Mutation Control
```
∀ key ∈ {Attester, Suspended, Paused, MaxAttesters},
  can_mutate(key)  ⟹  caller = Admin
```

**Verification:** Kani access control analysis
**Property:** Only authorized calls modify critical state
**Test:** Verify unauthorized callers revert on all mutation attempts

## Attestation Registry Invariants

### Invariant 4: Successful Attestation Requires Active Attester
```
attest(a, h) = Ok(id)  ⟹  is_attester(a) @ invocation_start = true
```

**Verification:** Kani intra-procedural analysis of attest() → is_attester() call
**Property:** No stale attestations by non-attesters
**Test:** Verify with attester revoked mid-call, concurrent suspensions

### Invariant 5: Get Attestation Returns Highest Sequence
```
get_attestation(h) = max{id : (h, id) ∈ history ∧ id.revoked = false}
```

**Verification:** Kani value-range analysis on history iteration
**Property:** Never returns revoked or outdated entries
**Test:** Verify with interleaved revocations and new attestations

### Invariant 6: History Length Bounded
```
|history[h]| ≤ MAX_HISTORY for all h
```

**Verification:** Kani loop bound analysis
**Property:** Old entries pruned; no unbounded growth
**Test:** Verify with max-size history, then add new attestation

## Multisig Account Invariants

### Invariant 7: Threshold Signatures Required
```
__check_auth(payload, signers) = Ok  ⟹  |{s ∈ signers : verify(s, payload) = true}| ≥ threshold
```

**Verification:** Kani signature count verification
**Property:** Can't bypass threshold with partial signatures
**Test:** Test 5-of-5, 3-of-5, 1-of-1 scenarios; verify rejection at threshold-1

## On-Chain Runtime Checks (`check_invariants()`)

Both registries expose a read-only `check_invariants() -> InvariantReport`
(`ok`, `checked`, `violations`) so monitors can verify consistency without
trusting an indexer. The `lafiya-watchdog` calls it every poll and raises a
critical `invariant_violation` alert when `violations` is non-empty.

| Violation symbol | Contract | Checks | Related |
|---|---|---|---|
| `admin_missing` | both | an admin is set | Invariant 3 |
| `count_exceeds_cap` | attester-registry | `AttesterCount <= MaxAttesters` | Invariant 2 |
| `schema_version_stale` | both | stored schema version equals the build's | upgrade runbook |
| `pending_admin_inconsistent` | both | `PendingAdmin` and its expiry are set together | Invariant 3 |
| `registry_wiring_invalid` | attestation-registry | the configured attester registry exposes the allowlist interface | Invariant 4 |
| `rate_limit_window_zero` | attestation-registry | an enabled rate limit has a non-zero window | - |
| `bond_accounting_mismatch` | attester-registry | `sum(bonds) + slashed_unswept == token.balance(contract)` | Invariant 8 |
| `vesting_reservation_mismatch` | incentive-pool | reserved liabilities include unvested and unclaimed grant amounts | Invariant 9 |

Every check is O(1) storage reads (CPU is asserted below 250k and 400k
instructions in the contract tests). Corruption tests inject bad state
through test-only storage writes and assert the matching symbol is reported.

## Bond Accounting Invariants (attester-registry)

### Invariant 8: Bond Custody Reconciliation
```
sum(bonds[a] for a ∈ Attesters) + slashed_unswept = token.balance(contract)
```

**Verification:** `bond_reconcile()` view plus `check_invariants()`; stateful
proptest over bond, unbond, slash, and sweep sequences (including a
fee-charging token), ≥10k cases in nightly
**Property:** The contract never custodies tokens it cannot account for; no bond
can be paid out twice and slashed funds are never double-counted
**Test:** Token balance deltas are asserted on deposit, withdraw, and slash,
mirroring the incentive-pool `reconcile` / `is_drift_detected` pattern. The
slash destination (treasury / burn / incentive pool) and the sweep authority are
explicit and covered by tests; see the bond ADR.

## Incentive Pool Vesting Invariants (incentive-pool)

### Invariant 9: Vesting Liabilities Are Reserved
```
reserved_liabilities ≥ sum(unvested(g) + unclaimed_vested(g) for g ∈ grants)
```

**Verification:** Stateful proptest over create, claim, and forfeit sequences,
including rounding; TLA+ model extended with vesting (if present)
**Property:** `withdraw` cannot drain funds backing outstanding vesting grants;
`claim_vested(g)` pays exactly the vested-but-unclaimed amount and never more
than `total`; forfeiting an unvested remainder returns it to the pool
**Test:** Linear vesting after the cliff, with boundary cases at start, cliff,
and end of duration

## Gaps Not Covered by Formal Verification

1. **Cross-contract calls:** Attestation registry calling attester registry (is_attester)
   - **Mitigation:** Proptest differential harness (#373)

2. **Concurrency under Soroban ledger:** Multiple transactions at same ledger height
   - **Mitigation:** State machine testing with concurrent operation sequences

3. **Rent/TTL behavior:** Storage expiration under archival
   - **Mitigation:** Integration test with TTL simulation

## Implementation Path

### Phase 1: Extract Pure Functions (Week 1)
Pull verification-friendly functions into `contracts/formal-verification/src/lib.rs`:
- `check_attester_consistency(storage)` → bool
- `check_count_integrity(storage)` → bool
- `check_auth_required(caller, key)` → bool

### Phase 2: Kani Specs (Week 2)
Write .rs model checker harnesses:
- `test_attester_existence_invariant` (Kani)
- `test_count_integrity` (Kani)
- `test_admin_only_mutation` (Kani)

### Phase 3: CI Integration (Week 3)
Add `.github/workflows/formal-verification.yml`:
```yaml
name: Formal Verification
on: [push, pull_request]
jobs:
  kani:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo kani --harness test_attester_existence_invariant --unwind 5
      - run: cargo kani --harness test_count_integrity --unwind 10
```

## References

- Kani model checker: https://model-checking.github.io/kani/
- Invariant-driven testing: https://en.wikipedia.org/wiki/Invariant_(mathematics)
- Multisig threshold: https://en.wikipedia.org/wiki/M-of-n_code
