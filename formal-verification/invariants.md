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
