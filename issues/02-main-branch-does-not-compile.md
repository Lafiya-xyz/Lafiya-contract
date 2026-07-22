---
title: "[bug]: main does not compile — attester-registry and attestation-registry both have build-breaking errors"
labels: bug, priority:p0
---

> **Blocks all other contract work.** Every architect-tier issue in this
> batch (revocation semantics, TTL policy, registry repoint) assumes a
> compiling baseline to reason about and test against. This should be
> fixed, or explicitly scheduled, before any of that work starts.
>
> Root cause: see the CI-workflow issue (#01) — `ci.yml` is invalid YAML
> and has never run a single job, so nothing has verified `main` compiles
> since whichever commit introduced the first of these breaks.

## Description

Reading `contracts/attester-registry/src/lib.rs` and
`contracts/attestation-registry/src/lib.rs` end to end turns up multiple
independent compile errors — undefined enum variants, a function that
falls through its return type, an undeclared event type, and public
methods the test suites call that don't exist on the contract. `cargo`
isn't available to confirm with a build in this environment, but each of
these is a straightforward `rustc`-level error, not a matter of
interpretation, and there are more than one, in more than one file, so
this isn't a single fat-fingered typo — the break has been accumulating.

### `contracts/attester-registry/src/lib.rs`

1. **Undefined `DataKey::Suspended` variant** (used at ~189, ~199, ~209).
   The `DataKey` enum (~12-24) only defines `Admin`, `PendingAdmin`,
   `Attester(Address)`, `SchemaVersion`. But `remove_attester`,
   `suspend_attester`, and `reinstate_attester` all read or write
   `DataKey::Suspended(attester)`, a variant that doesn't exist.

2. **`is_attester` doesn't compile** (~216-229). The function is declared
   to return `bool`, but its last statement is a `let` binding with no
   trailing expression:
   ```rust
   pub fn is_attester(env: Env, attester: Address) -> bool {
       let is_allowlisted = env
           .storage()
           .persistent()
           .get(&DataKey::Attester(attester.clone()))
           .unwrap_or(false);          // <-- type mismatch, see below
       if !is_allowlisted {
           return false;
       }
       let is_suspended = env
           .storage()
           .persistent()
           .has(&DataKey::Attester(attester))
       // <-- no `;`, no return, no trailing value — fn falls off the end
   }
   ```
   Independently, `.get(&DataKey::Attester(...))` is typed as
   `Option<AttesterInfo>` everywhere else in this file (e.g.
   `add_attester`), so `.unwrap_or(false)` is a type mismatch — `bool` is
   not `AttesterInfo`. Fixing just the missing `return` isn't enough; the
   allowlisted/suspended check itself needs to be rewritten (this also
   overlaps with the "suspend vs. allowlist state model" question raised
   in the revocation-semantics issue — worth resolving together).

3. **Undeclared event type `Upgraded`** (~256-262). `upgrade()` does
   `Upgraded { new_wasm_hash }.publish(&env)`, but no `Upgraded` struct is
   declared anywhere in the file (compare to `AttesterAdded`,
   `AttesterSuspended`, etc., which are all declared with
   `#[contractevent]` above the `impl` block).

4. **`get_admin()` doesn't exist but is called from tests.**
   `contracts/attester-registry/src/test.rs` calls `client.get_admin()`
   (line 30) and `client.try_get_admin()` (line 37). There is no public
   `get_admin` function in `lib.rs` — only a private `fn admin(env: &Env)
   -> Result<Address, Error>` helper. The generated `AttesterRegistryClient`
   has no such method, so the test crate fails to compile independently of
   the three errors above.

### `contracts/attestation-registry/src/lib.rs`

5. **Undefined `Error` variants.** The `Error` enum (~77-85) defines only
   `NotInitialized, AlreadyInitialized, AttesterNotAllowlisted,
   NoPendingTransfer`. But `attest()` (~160) returns
   `Error::InvalidRegistryWiring` and `revoke_attestation()` (~203)
   returns `Error::AttestationNotFound` — neither variant exists.

6. **`get_admin()` / `get_attester_registry()` don't exist but are called
   from tests.** `contracts/attestation-registry/src/test.rs` calls
   `client.get_admin()` and `client.try_get_admin()` (lines 34, 44), and
   `client.get_attester_registry()` / `client.try_get_attester_registry()`
   (lines 35, 46). Neither function is defined in `lib.rs`.

7. **`Self::attester_registry(&env)` doesn't exist.** `attest()` (~156)
   calls `let registry_id = Self::attester_registry(&env)?;` to look up
   the configured `attester-registry` address. No `fn attester_registry`
   is defined anywhere in the file — only `fn admin(env: &Env) ->
   Result<Address, Error>` exists as a private helper, and it returns the
   wrong `DataKey` for this purpose even if the name were fixed
   (`DataKey::Admin`, not `DataKey::AttesterRegistry`). This is a fourth,
   independent compile error in this file — `attest()`, the contract's
   core function, cannot build at all as written. Confirmed with `grep -n
   "fn attester_registry\|Self::attester_registry"
   contracts/attestation-registry/src/lib.rs`, which only matches the
   call site, never a definition.

## Expected behavior

`cargo test --workspace` and `cargo build --workspace --release --target
wasm32v1-none` both succeed, as `make check` and `CONTRIBUTING.md` imply
they should on `main` at all times.

## Actual behavior

Both crates fail to compile for the reasons above; the test crates for
both contracts additionally fail because they call client methods that
don't exist on the contract's public interface.

## Suggested fix shape (not prescriptive — needs a maintainer decision)

- Add the missing `Suspended(Address)` `DataKey` variant.
- Rewrite `is_attester` to actually implement "allowlisted AND NOT
  suspended," returning a `bool` correctly — this is also where the
  suspend/allowlist state-model question from the revocation-semantics
  issue should get resolved, so don't treat this as a pure syntax patch.
- Declare `Upgraded` as a `#[contractevent]` (or drop the event if it
  wasn't meant to ship yet).
- Decide whether `get_admin()` (both contracts) and
  `get_attester_registry()` (attestation-registry) are meant to be public
  read APIs — the tests clearly assume they are — and add them, rather
  than changing the tests to match a narrower interface, since a way to
  read the configured admin/registry off-chain is generally useful for
  the CLI and indexer.
- Add the missing `Error::InvalidRegistryWiring` and
  `Error::AttestationNotFound` variants to `attestation-registry`'s
  `Error` enum, and add them to `docs/error-codes.md`
  (`test_error_codes_are_documented` in `attestation-registry/src/test.rs`
  will enforce this once the crate compiles at all).
- Add a private `fn attester_registry(env: &Env) -> Result<Address,
  Error>` helper (mirroring the existing `admin` helper) that reads
  `DataKey::AttesterRegistry` and returns `Error::NotInitialized` if
  unset, and call it from `attest()` in place of the current dangling
  `Self::attester_registry(&env)` reference.

## Acceptance criteria

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo test --workspace` succeeds, including
      `test_error_codes_are_documented` in both contracts.
- [ ] `cargo build --workspace --release --target wasm32v1-none` succeeds.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
      (both crates use `#![deny(clippy::unwrap_used, clippy::expect_used,
      clippy::panic)]`, so the `is_attester` rewrite needs to respect
      that).
- [ ] CI (once #01 is fixed) is green on the PR that fixes this.

## Environment

- Contract(s) affected: attester-registry, attestation-registry
- Verified by reading source, not by `cargo build` — `cargo` is not
  available in the environment this audit was performed in. A maintainer
  with a working toolchain should confirm with an actual build before
  starting the fix, in case anything above is stale relative to
  uncommitted local state.
