---
title: "[audit] BUILD-01: main fails to compile — 7 independent errors across both registry contracts"
labels: bug, priority:p0, severity:critical
---

**Severity:** Critical
**Difficulty:** Trivial (always active on any build of `main`)
**Type:** Build Integrity / Correctness

> Blocks all downstream work. Enabled by CI-01 (no CI has ever run to
> catch this). Every architecture-tier finding in this batch assumes a
> compiling baseline to implement and test against.

## Summary

`contracts/attester-registry/src/lib.rs` and
`contracts/attestation-registry/src/lib.rs` each contain multiple
independent, unrelated compile errors: undefined enum variants, a
function that falls through its declared return type, an undeclared
event type, an undefined helper-function call, and public methods that
the contracts' own test suites call but that do not exist on the
generated client. Verified by reading source line-by-line; `cargo` is
unavailable in this audit environment, so a maintainer with a working
toolchain should confirm with `cargo build --workspace` before starting
remediation, in case local state differs from what was reviewed.

## Findings

### BUILD-01a — `DataKey::Suspended` referenced but never declared

**Location:** `contracts/attester-registry/src/lib.rs:12-24` (enum
declaration), used at lines `189`, `199`, `209`.

The `DataKey` enum declares only `Admin`, `PendingAdmin`,
`Attester(Address)`, `SchemaVersion`. `remove_attester` (line 189),
`suspend_attester` (line 199), and `reinstate_attester` (line 209) all
reference `DataKey::Suspended(attester)` — an undefined variant.

### BUILD-01b — `is_attester` has no valid return path and a type mismatch

**Location:** `contracts/attester-registry/src/lib.rs:216-229`

```rust
216	    pub fn is_attester(env: Env, attester: Address) -> bool {
217	        let is_allowlisted = env
218	            .storage()
219	            .persistent()
220	            .get(&DataKey::Attester(attester.clone()))
221	            .unwrap_or(false);
222	        if !is_allowlisted {
223	            return false;
224	        }
225	        let is_suspended = env
226	            .storage()
227	            .persistent()
228	            .has(&DataKey::Attester(attester))
229	    }
```

Two independent errors:
1. Line 229 closes the function on a `let` binding with no trailing
   semicolon and no returned expression — the function's declared return
   type is `bool`, but this path yields `()`.
2. Line 221: `.get(&DataKey::Attester(...))` is typed `Option<AttesterInfo>`
   everywhere else this key is used (e.g. `add_attester`, line ~154).
   `.unwrap_or(false)` requires the `Option`'s inner type to be `bool` —
   `AttesterInfo` is not `bool`. This is a type error independent of (1).

### BUILD-01c — `Upgraded` event published but never declared

**Location:** `contracts/attester-registry/src/lib.rs:256-262`

```rust
256	    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
257	        Self::admin(&env)?.require_auth();
258	        env.deployer()
259	            .update_current_contract_wasm(new_wasm_hash.clone());
260	        Upgraded { new_wasm_hash }.publish(&env);
261	        Ok(())
262	    }
```

No `Upgraded` struct exists in this file. Every other emitted event
(`AttesterAdded`, `AttesterSuspended`, etc.) is declared with
`#[contractevent]` above the `impl` block; `Upgraded` is not.

### BUILD-01d — `get_admin()` called by tests, not defined on the contract

**Location:** `contracts/attester-registry/src/test.rs:30,37` call
`client.get_admin()` / `client.try_get_admin()`. No public `get_admin`
function exists in `contracts/attester-registry/src/lib.rs` — only a
private `fn admin(env: &Env) -> Result<Address, Error>` helper. The
generated `AttesterRegistryClient` has no `get_admin` method; the test
crate fails to compile independent of BUILD-01a–c.

### BUILD-01e — Undefined `Error` variants referenced in attestation-registry

**Location:** `contracts/attestation-registry/src/lib.rs:77-85` (enum),
used at lines `160`, `203`.

The `Error` enum declares only `NotInitialized`, `AlreadyInitialized`,
`AttesterNotAllowlisted`, `NoPendingTransfer`. `attest()` (line 160)
returns `Error::InvalidRegistryWiring`; `revoke_attestation()` (line 203)
returns `Error::AttestationNotFound`. Neither variant is declared.

### BUILD-01f — `get_admin()` / `get_attester_registry()` called by tests, not defined

**Location:** `contracts/attestation-registry/src/test.rs:34-35,44,46`
call `client.get_admin()`, `client.get_attester_registry()`,
`client.try_get_admin()`, `client.try_get_attester_registry()`. Neither
function is defined in `contracts/attestation-registry/src/lib.rs`.

### BUILD-01g — `Self::attester_registry(&env)` called, never defined

**Location:** `contracts/attestation-registry/src/lib.rs:156`

```rust
149	    pub fn attest(
150	        env: Env,
151	        attester: Address,
152	        record_hash: BytesN<32>,
153	    ) -> Result<Attestation, Error> {
154	        attester.require_auth();
155	
156	        let registry_id = Self::attester_registry(&env)?;
```

`attest()` — the contract's core, most-called function — calls
`Self::attester_registry(&env)?` to resolve the configured
`attester-registry` address. No such method exists anywhere in the file;
only `fn admin(env: &Env) -> Result<Address, Error>` is defined, and it
reads the wrong storage key (`DataKey::Admin`) even if reused by name.
Confirmed via `grep -n "fn attester_registry\|Self::attester_registry"
contracts/attestation-registry/src/lib.rs`, which matches only the call
site.

## Impact

Both contracts fail `cargo build`. Both test crates additionally fail
independent of the `lib.rs` errors, since they call client methods that
were never implemented. `attest()` — the sole function that records an
attestation — cannot compile at all (BUILD-01g), meaning the
attestation-registry contract has never successfully built with its core
write path present, as currently written on `main`.

## Recommendation

- Add the missing `Suspended(Address)` `DataKey` variant.
- Rewrite `is_attester` to implement "allowlisted AND NOT suspended,"
  returning `bool` correctly. Coordinate with the revocation-semantics
  finding (ARCH-02) before finalizing this logic — it's the same state
  model.
- Declare `Upgraded` as `#[contractevent]`, or remove the emit if the
  event wasn't meant to ship.
- Decide whether `get_admin()` / `get_attester_registry()` are intended
  public read APIs (the tests assume they are) and implement them, rather
  than narrowing the tests to match an accidental gap — off-chain tooling
  (CLI, indexer) benefits from a supported read path for this
  configuration state.
- Add `Error::InvalidRegistryWiring` and `Error::AttestationNotFound` to
  `attestation-registry`'s `Error` enum and to `docs/error-codes.md`
  (`test_error_codes_are_documented` in `attestation-registry/src/test.rs`
  enforces this once the crate compiles).
- Add a private `fn attester_registry(env: &Env) -> Result<Address,
  Error>` helper mirroring `admin`, reading `DataKey::AttesterRegistry`,
  returning `Error::NotInitialized` if unset; call it from `attest()` in
  place of the current dangling reference.

## Verification

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo test --workspace` succeeds, including
      `test_error_codes_are_documented` in both contracts.
- [ ] `cargo build --workspace --release --target wasm32v1-none` succeeds.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
      (both crates carry `#![deny(clippy::unwrap_used, clippy::expect_used,
      clippy::panic)]`; the `is_attester` rewrite must respect that).
- [ ] CI-01 is fixed and shows green on the PR that fixes this.
