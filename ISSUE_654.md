# Issue #654: [Advanced] attestation-registry: session-key attestation path is broken at the type and interface level

## 🔍 Problem Summary

The `attest_with_session` function in the attestation-registry contract cannot work as written. It has two critical type and interface mismatches:

1. **Wrong number of arguments**: Calls `record_attestation` with 6 arguments when it requires 7
2. **Interface mismatch**: The cross-contract call to `is_session_valid` has the wrong signature and return type compared to the actual attester-registry contract

This breaks the delegated session-key flow end to end, preventing relayers from submitting attestations on behalf of attesters using session keys.

## 🐛 Current Behavior

### Issue 1: Incorrect `record_attestation` Call

**Location**: `contracts/attestation-registry/src/lib.rs:1117`

```rust
pub fn attest_with_session(
    env: Env,
    session_pk: Address,
    patient: Address,
    record_hash: BytesN<32>,
) -> Result<Attestation, Error> {
    session_pk.require_auth();
    Self::require_not_paused(&env)?;

    let attester = Self::resolve_session(&env, &session_pk)?;
    Self::require_allowlisted_attester(&env, &attester)?;
    // ❌ WRONG: Passing 6 arguments
    Self::record_attestation(&env, attester, patient, record_hash, 0, None)
}
```

**Actual signature** (line 2607):
```rust
fn record_attestation(
    env: &Env,
    attester: Address,
    patient: Address,
    record_hash: BytesN<32>,
    commitment_version: u32,
    previous_record_hash: Option<BytesN<32>>,
    scope: Option<AttestationScope>,  // ❌ MISSING ARGUMENT
) -> Result<Attestation, Error>
```

The function requires 7 arguments but only 6 are provided. The missing argument is `scope: Option<AttestationScope>`.

### Issue 2: Interface Signature Mismatch

**Location**: `contracts/attestation-registry/src/lib.rs:41-52`

**Declared interface** (what attestation-registry expects):
```rust
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_session_valid(
        env: Env,
        session_pk: Address,      // ❌ WRONG TYPE
        scope: Symbol,
    ) -> Result<Address, u32>;    // ❌ WRONG RETURN TYPE
}
```

**Actual implementation** in attester-registry (`contracts/attester-registry/src/lib.rs:1512`):
```rust
pub fn is_session_valid(
    env: Env,
    session_pk: BytesN<32>,       // ✅ CORRECT: BytesN<32>
    scope: Symbol
) -> Result<(), Error>            // ✅ CORRECT: Result<(), Error>
```

**Three mismatches**:
1. Parameter type: `Address` vs `BytesN<32>`
2. Return type on success: Returns `Address` vs returns `()`
3. Return type on error: Returns `u32` vs returns `Error`

**Current broken resolve_session** (line 2372):
```rust
fn resolve_session(env: &Env, session_pk: &Address) -> Result<Address, Error> {
    let registry_id = Self::attester_registry(env)?;
    let registry = AttesterRegistryClient::new(env, &registry_id);
    let scope = Symbol::new(env, "attest");
    // ❌ Type mismatch: session_pk is &Address but needs BytesN<32>
    // ❌ Return type mismatch: expects Result<Address, u32> but gets Result<(), Error>
    match registry.try_is_session_valid(session_pk, &scope) {
        Ok(Ok(attester)) => Ok(attester),  // ❌ No attester returned
        Ok(Err(code)) => Err(Self::map_session_error(code)),  // ❌ Wrong error type
        _ => Err(Error::AttesterRegistryUnavailable),
    }
}
```

## 💡 Root Cause Analysis

The attestation-registry contract was developed with an **outdated or incorrect understanding** of the attester-registry's session key API:

1. The interface declaration doesn't match the actual implementation
2. The session key type changed from `Address` to `BytesN<32>` for cryptographic correctness
3. The return semantics changed from returning the attester address to returning `()` on success
4. The actual attester is stored in the `SessionKey` struct and must be retrieved separately

## ✅ Solution Design

### Fix 1: Correct the Interface Declaration

Update `AttesterRegistryInterface` to match the actual attester-registry implementation:

```rust
#[contractclient(name = "AttesterRegistryClient")]
pub trait AttesterRegistryInterface {
    fn is_attester(env: Env, attester: Address) -> bool;
    fn is_attester_for_region(env: Env, attester: Address, region: String) -> bool;
    fn get_attester_trust_revoked_after(env: Env, attester: Address) -> Option<u64>;
    
    /// ✅ FIXED: Correct signature matching attester-registry
    fn is_session_valid(
        env: Env,
        session_pk: BytesN<32>,   // Changed from Address
        scope: Symbol,
    ) -> Result<(), Error>;        // Changed from Result<Address, u32>
    
    /// ✅ NEW: Add method to retrieve session key data
    fn get_session_key(env: Env, session_pk: BytesN<32>) -> Option<SessionKey>;
}
```

### Fix 2: Import SessionKey Type

Since we need to work with `SessionKey`, we must either:
- Import it from attester-registry
- Declare a matching struct locally

**Recommended approach**: Declare locally to avoid tight coupling:

```rust
/// Mirror of attester-registry's SessionKey for cross-contract calls
#[derive(Clone)]
#[contracttype]
pub struct SessionKey {
    pub attester: Address,
    pub scope: Symbol,
    pub expires_at: u64,
}
```

### Fix 3: Rewrite resolve_session Method

```rust
/// Resolve a delegated session key to its owning attester, validating
/// that it is unexpired and permitted to perform "attest" scope.
fn resolve_session(env: &Env, session_pk: &BytesN<32>) -> Result<Address, Error> {
    let registry_id = Self::attester_registry(env)?;
    let registry = AttesterRegistryClient::new(env, &registry_id);
    let scope = Symbol::new(env, "attest");
    
    // ✅ Step 1: Validate the session is valid
    match registry.try_is_session_valid(session_pk, &scope) {
        Ok(Ok(())) => {}, // Session is valid, continue
        Ok(Err(err)) => return Err(Self::map_session_error_from_registry(err)),
        Err(_) => return Err(Error::AttesterRegistryUnavailable),
    }
    
    // ✅ Step 2: Retrieve session data to get the attester
    match registry.try_get_session_key(session_pk) {
        Ok(Some(session)) => Ok(session.attester),
        Ok(None) => Err(Error::SessionNotFound),
        Err(_) => Err(Error::AttesterRegistryUnavailable),
    }
}
```

### Fix 4: Update map_session_error

Since `Error` type is now returned instead of `u32` codes, we need a new mapper:

```rust
/// Map a registry Error to our contract's session error types.
fn map_session_error_from_registry(err: attester_registry::Error) -> Error {
    match err {
        attester_registry::Error::SessionExpired => Error::SessionExpired,
        attester_registry::Error::SessionScopeMismatch => Error::SessionScopeMismatch,
        attester_registry::Error::SessionNotFound => Error::SessionNotFound,
        attester_registry::Error::AttesterNotFound => Error::AttesterNotAllowlisted,
        _ => Error::SessionNotFound, // Default for unexpected errors
    }
}
```

**Alternative**: Keep numeric codes if Error types aren't accessible:

```rust
/// Map a registry session error to a distinct contract error.
/// Error enum discriminants from attester-registry:
/// SessionNotFound = 25, SessionExpired = 26, SessionScopeMismatch = 27
fn map_session_error_from_registry(err: attester_registry::Error) -> Error {
    // Convert Error to discriminant if needed
    let code = err as u32;
    match code {
        26 => Error::SessionExpired,
        27 => Error::SessionScopeMismatch,
        25 => Error::SessionNotFound,
        _ => Error::SessionNotFound,
    }
}
```

### Fix 5: Update attest_with_session Parameter Type

```rust
/// Attest using a delegated session key. The session key's owning attester
/// becomes the recorded attester, but the session key itself must authorize
/// the transaction. This allows relayers to submit on behalf of attesters.
pub fn attest_with_session(
    env: Env,
    session_pk: BytesN<32>,  // ✅ Changed from Address to BytesN<32>
    patient: Address,
    record_hash: BytesN<32>,
) -> Result<Attestation, Error> {
    // Session key must authorize (not the attester)
    // ❌ PROBLEM: BytesN<32> cannot require_auth()
    // We need to use an Address-wrapped session key OR
    // the attester-registry needs to handle auth differently
    
    // ✅ SOLUTION: Accept both the session key data AND its address
    // OR rely on the registry's validation that was already done
}
```

**Better approach**: Accept session key as Address for auth, convert internally:

```rust
pub fn attest_with_session(
    env: Env,
    session_pk: Address,     // Keep as Address for require_auth()
    patient: Address,
    record_hash: BytesN<32>,
) -> Result<Attestation, Error> {
    session_pk.require_auth();
    Self::require_not_paused(&env)?;
    
    // ✅ Convert Address to BytesN<32> for registry call
    // Assuming the session key address is derived from the public key bytes
    let session_bytes: BytesN<32> = session_pk.try_into()
        .map_err(|_| Error::InvalidSessionKey)?;
    
    let attester = Self::resolve_session(&env, &session_bytes)?;
    Self::require_allowlisted_attester(&env, &attester)?;
    
    // ✅ FIXED: Pass all 7 required arguments including scope
    Self::record_attestation(
        &env,
        attester,
        patient,
        record_hash,
        0,                    // commitment_version
        None,                 // previous_record_hash
        None,                 // scope: no attestation scope restriction
    )
}
```

### Fix 6: Add Missing Error Variants

Ensure these error variants exist in the Error enum:

```rust
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    // ... existing variants ...
    SessionNotFound = 48,
    SessionExpired = 49,
    SessionScopeMismatch = 50,
    InvalidSessionKey = 51,  // ✅ NEW: for conversion failures
    // ... other variants ...
}
```

## 📋 Implementation Plan

### Phase 1: Type Fixes
1. ✅ Update `AttesterRegistryInterface` signature for `is_session_valid`
2. ✅ Add `get_session_key` method to interface
3. ✅ Declare `SessionKey` struct locally or import it
4. ✅ Add `InvalidSessionKey` error variant if needed

### Phase 2: resolve_session Rewrite
5. ✅ Update `resolve_session` to use correct types
6. ✅ Implement two-step validation: check validity, then fetch attester
7. ✅ Replace `map_session_error` with version that handles Error type

### Phase 3: attest_with_session Fix
8. ✅ Determine session key representation strategy (Address vs BytesN<32>)
9. ✅ Add address-to-bytes conversion if needed
10. ✅ Fix `record_attestation` call to include all 7 arguments
11. ✅ Pass `None` for `scope` parameter (or appropriate value)

### Phase 4: Testing
12. ✅ Create cross-contract integration test
13. ✅ Test session key creation in attester-registry
14. ✅ Test successful attestation with valid session
15. ✅ Test rejection of expired session
16. ✅ Test rejection of wrong scope
17. ✅ Test rejection of non-existent session
18. ✅ Test that correct attester is recorded

## 🧪 Test Strategy

### Unit Tests

```rust
#[test]
fn test_resolve_session_valid() {
    let (env, attest_reg, attester_reg, admin) = setup_with_session();
    
    // Create a session key
    let attester = Address::generate(&env);
    let session_pk = BytesN::from_array(&env, &[1u8; 32]);
    
    attester_reg.register_attester(&attester, /* ... */);
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "attest"),
        env.ledger().timestamp() + 86400,
    );
    
    // Should resolve to attester
    let resolved = attest_reg.resolve_session(&session_pk);
    assert_eq!(resolved, attester);
}

#[test]
#[should_panic(expected = "SessionExpired")]
fn test_resolve_session_expired() {
    let (env, attest_reg, attester_reg, admin) = setup_with_session();
    
    let attester = Address::generate(&env);
    let session_pk = BytesN::from_array(&env, &[1u8; 32]);
    
    attester_reg.register_attester(&attester, /* ... */);
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "attest"),
        env.ledger().timestamp() + 100,
    );
    
    // Advance time past expiration
    env.ledger().with_mut(|li| li.timestamp += 200);
    
    // Should fail with SessionExpired
    attest_reg.resolve_session(&session_pk);
}

#[test]
#[should_panic(expected = "SessionScopeMismatch")]
fn test_resolve_session_wrong_scope() {
    let (env, attest_reg, attester_reg, admin) = setup_with_session();
    
    let attester = Address::generate(&env);
    let session_pk = BytesN::from_array(&env, &[1u8; 32]);
    
    attester_reg.register_attester(&attester, /* ... */);
    // Create session for "revoke" scope
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "revoke"),  // Wrong scope
        env.ledger().timestamp() + 86400,
    );
    
    // Should fail with SessionScopeMismatch
    attest_reg.resolve_session(&session_pk);
}
```

### Integration Tests

```rust
#[test]
fn test_attest_with_session_end_to_end() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[0x42u8; 32]);
    
    // Setup registries
    let attester_reg_id = env.register(
        attester_registry::AttesterRegistry,
        (admin.clone(),)
    );
    let attester_reg = attester_registry::AttesterRegistryClient::new(
        &env,
        &attester_reg_id
    );
    
    let attest_reg_id = env.register(
        AttestationRegistry,
        (admin.clone(), attester_reg_id.clone())
    );
    let attest_reg = AttestationRegistryClient::new(&env, &attest_reg_id);
    
    // Setup roles and register attester
    attester_reg.grant_role(&attester_registry::Role::Registrar, &admin);
    attester_reg.grant_role(&attester_registry::Role::Guardian, &admin);
    attester_reg.register_attester(
        &attester,
        &String::from_str(&env, "USA"),
        &String::from_str(&env, "Hospital A"),
        None,
        None,
    );
    
    // Create session key
    let session_pk = BytesN::from_array(&env, &[0xAAu8; 32]);
    let session_addr = Address::from_contract_id(&env, &session_pk);
    
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "attest"),
        env.ledger().timestamp() + 86400,
    );
    
    // Grant consent
    attest_reg.grant_consent(&patient, &attester, None, None);
    
    // ✅ Attest using session key
    let attestation = attest_reg.attest_with_session(
        &session_addr,
        &patient,
        &record_hash,
    );
    
    // Verify attestation was recorded with correct attester
    assert_eq!(attestation.attester, attester);
    assert!(attestation.timestamp > 0);
    assert_eq!(attestation.commitment_version, 0);
    
    // Verify attestation is retrievable
    let history = attest_reg.get_attestations(&record_hash);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().attester, attester);
}

#[test]
fn test_attest_with_session_relayer_pays_fees() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let attester = Address::generate(&env);
    let relayer = Address::generate(&env);
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[0x42u8; 32]);
    
    // ... setup as above ...
    
    // The key point: relayer submits the transaction
    // but attester is the one being recorded
    env.set_auths(&[
        soroban_sdk::testutils::MockAuth {
            address: &session_addr,
            invoke: &MockAuthInvoke {
                contract: &attest_reg_id,
                fn_name: "attest_with_session",
                args: (session_addr.clone(), patient.clone(), record_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        },
    ]);
    
    let attestation = attest_reg.attest_with_session(
        &session_addr,
        &patient,
        &record_hash,
    );
    
    // Attester is recorded, not relayer
    assert_eq!(attestation.attester, attester);
}

#[test]
#[should_panic(expected = "SessionExpired")]
fn test_attest_with_expired_session() {
    let (env, attest_reg, attester_reg, attester, session_pk) = setup();
    
    // Create session that expires soon
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "attest"),
        env.ledger().timestamp() + 100,
    );
    
    // Advance past expiration
    env.ledger().with_mut(|li| li.timestamp += 200);
    
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[0x42u8; 32]);
    
    // Should fail
    attest_reg.attest_with_session(
        &Address::from_contract_id(&env, &session_pk),
        &patient,
        &record_hash,
    );
}

#[test]
#[should_panic(expected = "SessionScopeMismatch")]
fn test_attest_with_wrong_scope_session() {
    let (env, attest_reg, attester_reg, attester, session_pk) = setup();
    
    // Create session for different scope
    attester_reg.create_session_key(
        &attester,
        &session_pk,
        &Symbol::new(&env, "revoke"),  // Wrong scope
        env.ledger().timestamp() + 86400,
    );
    
    let patient = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[0x42u8; 32]);
    
    // Should fail
    attest_reg.attest_with_session(
        &Address::from_contract_id(&env, &session_pk),
        &patient,
        &record_hash,
    );
}
```

### Test File Organization

Create new test file: `contracts/attestation-registry/src/session_test.rs`

```rust
#![cfg(test)]
extern crate std;

use crate::{
    AttestationRegistry, AttestationRegistryClient, Error,
};
use attester_registry;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, BytesN, Env, String, Symbol,
};

mod session_integration {
    use super::*;
    
    // All session-related integration tests here
}
```

Add to `lib.rs`:
```rust
#[cfg(test)]
mod session_test;
```

## 🔒 Security Considerations

### 1. Session Key Authorization
- **Concern**: Session key must prove authorization, not the attester
- **Mitigation**: `session_pk.require_auth()` ensures the session key holder authorized the call
- **Verification**: Integration tests confirm relayer can submit without attester signature

### 2. Session Validation
- **Concern**: Expired or invalid sessions must not be usable
- **Mitigation**: Two-step validation: `is_session_valid` checks expiry and scope
- **Verification**: Tests confirm expired and wrong-scope sessions are rejected

### 3. Attester Allowlist Check
- **Concern**: Delegated attester must still be allowlisted
- **Mitigation**: `require_allowlisted_attester` called after resolving session
- **Verification**: Test confirms removed attesters' sessions don't work

### 4. Rate Limiting
- **Concern**: Session-based attestations must respect rate limits
- **Mitigation**: `record_attestation` applies rate limits to the resolved attester
- **Verification**: Rate limit tests cover session attestations

### 5. Consent Requirements
- **Concern**: Session attestations must still require patient consent
- **Mitigation**: `consume_consent` checks in `record_attestation` apply to all paths
- **Verification**: Tests confirm consent is required for session attestations

### 6. Type Safety
- **Concern**: Type mismatches could cause runtime panics or incorrect behavior
- **Mitigation**: Strong typing with BytesN<32> for cryptographic keys
- **Verification**: Compile-time type checking, conversion error handling

## 📊 Acceptance Criteria

### Core Functionality
- [ ] `attest_with_session` compiles without errors
- [ ] `attest_with_session` successfully records attestation with valid session
- [ ] Recorded attestation shows the delegating attester, not session key
- [ ] Session key holder can authorize (require_auth passes)
- [ ] `record_attestation` receives all 7 required arguments

### Cross-Contract Integration
- [ ] `is_session_valid` call succeeds with matching types
- [ ] `get_session_key` call retrieves session data correctly
- [ ] Error mapping handles all session error variants
- [ ] Registry unavailability handled gracefully

### Session Validation
- [ ] Expired sessions are rejected with `SessionExpired` error
- [ ] Wrong scope sessions are rejected with `SessionScopeMismatch` error
- [ ] Non-existent sessions are rejected with `SessionNotFound` error
- [ ] Sessions for removed attesters are rejected

### Existing Attestation Rules
- [ ] Allowlist check still applies to resolved attester
- [ ] Rate limits apply to resolved attester
- [ ] Probation caps apply to resolved attester
- [ ] Consent requirements still enforced
- [ ] All other attestation validations unchanged

### Test Coverage
- [ ] Unit tests for `resolve_session` with valid session
- [ ] Unit tests for all session error cases
- [ ] Integration test for successful end-to-end flow
- [ ] Integration test for relayer-submitted attestation
- [ ] Integration tests for all error paths
- [ ] All existing tests continue to pass

### Documentation
- [ ] Function documentation updated for `attest_with_session`
- [ ] Interface documentation clarifies session key flow
- [ ] Error variants documented
- [ ] Example usage provided in comments or docs

## 📝 Implementation Notes

### Address vs BytesN<32> Decision

The core tension is:
- **Soroban auth** requires `Address` type for `require_auth()`
- **Session keys** are cryptographic keys, naturally `BytesN<32>`
- **attester-registry** uses `BytesN<32>` for session keys

**Recommended solution**:
1. Accept `Address` parameter for auth purposes
2. Convert to `BytesN<32>` internally for registry calls
3. Document that session key Address must be derivable from key bytes

**Alternative**: Change attester-registry to accept Address, but this may break existing session key logic.

### Scope Parameter Decision

The `scope` parameter in `record_attestation` appears to be for `AttestationScope` (facility/region restrictions), not session scope validation.

**Recommendation**: Pass `None` for session-based attestations unless there's a specific scope to enforce.

### Error Code Mapping

If `attester_registry::Error` type isn't accessible for pattern matching:
- Use error code discriminants (25, 26, 27)
- Document the mapping clearly
- Consider requesting a public error code conversion method in attester-registry

### SessionKey Struct

If `SessionKey` struct isn't exported from attester-registry:
- Declare a matching struct locally
- Keep in sync with attester-registry definition
- Consider requesting it be made public in attester-registry

## 🔗 Dependencies

### Code Files to Modify
1. `contracts/attestation-registry/src/lib.rs`
   - Lines 41-52: AttesterRegistryInterface trait
   - Lines 1105-1118: attest_with_session function
   - Lines 2371-2399: resolve_session function
   - Add SessionKey struct definition (if not imported)
   - Add/update error handling functions

2. `contracts/attestation-registry/src/error.rs` (if separate)
   - Add `InvalidSessionKey` error variant if needed

3. `contracts/attestation-registry/src/session_test.rs` (new file)
   - All session integration tests

### External Dependencies
- **attester-registry contract**: Must be deployed and functional
- **Session key creation**: Requires attester-registry `create_session_key` function
- **Test environment**: Requires both contracts registered in test environment

### Build Dependencies
- No new crate dependencies required
- Existing `soroban-sdk` provides all needed types

## 🚀 Deployment Considerations

### Breaking Changes
- **API change**: `attest_with_session` parameter type may change
- **Existing deployments**: Will need contract upgrade
- **Clients**: May need to update call signatures

### Migration Strategy
1. Deploy updated attestation-registry contract
2. Test session attestation flow on testnet
3. Notify clients of parameter type changes
4. Deploy to mainnet after verification

### Backwards Compatibility
- Existing attestation functions unchanged
- Only session-based attestation affected
- No data migration needed

## 📚 Additional Context

### Session Key Flow Overview
```
1. Attester creates session key in attester-registry
   └─> Specifies scope (e.g., "attest"), expiration time

2. Attester shares session key with relayer (off-chain)

3. Relayer calls attest_with_session
   ├─> Session key signs the transaction (require_auth)
   ├─> attestation-registry calls attester-registry
   │   ├─> is_session_valid: validates expiry + scope
   │   └─> get_session_key: retrieves owning attester
   ├─> Resolved attester checked against allowlist
   ├─> Rate limits applied to attester
   └─> Attestation recorded with attester as signer

4. Result: Attestation attributed to attester, fees paid by relayer
```

### Benefits of Session Keys
1. **Gas efficiency**: Relayers can batch multiple attestations
2. **Privacy**: Attester's key not exposed in every transaction
3. **Convenience**: Attesters don't need to sign each attestation
4. **Delegation**: Authorized third parties can submit on behalf
5. **Revocability**: Sessions can be revoked without changing attester key

### Related Issues
- Session key creation in attester-registry (prerequisite)
- Session key revocation (separate feature)
- Batch attestation with sessions (future enhancement)

## ✅ Definition of Done

1. All type mismatches resolved
2. Interface matches actual implementation
3. `record_attestation` called with correct number of arguments
4. All unit tests pass
5. All integration tests pass
6. No compiler errors or warnings
7. Documentation updated
8. Code review completed
9. Testnet deployment verified
10. Performance benchmarks acceptable

---

**Issue Type**: Bug Fix / Type Correction
**Priority**: High (Critical path is broken)
**Complexity**: Medium (Requires careful type handling and cross-contract testing)
**Estimated Effort**: 4-6 hours (implementation + comprehensive testing)
**Risk Level**: Medium (Cross-contract changes, needs thorough testing)
