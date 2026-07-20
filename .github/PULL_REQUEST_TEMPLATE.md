## Summary

What does this PR change, and why?

Closes # <!-- issue number -->

## Checklist

The expectations below come from [CONTRIBUTING.md](CONTRIBUTING.md) —
see it for the full workflow and rationale.

- [ ] `make check` passes locally (`fmt` + `clippy` + `test` + wasm
      build — the same steps CI runs)
- [ ] Tests added or updated: every new contract function has unit tests
      covering both the success path and the failure/authorization paths
      (see `contracts/*/src/test.rs` for patterns)
- [ ] `CHANGELOG.md` updated under `Unreleased`
- [ ] `Cargo.lock` is committed and up to date
- [ ] Docs updated (`README.md`, `docs/`) if behavior, error codes, or
      workflow changed
- [ ] Cross-contract calls go through a `#[contractclient]` trait
      interface, not a direct crate dependency
- [ ] No real personal or health data (including test fixtures that
      could be mistaken for it) is included in this change

## Schema / cross-repo impact

- [ ] This PR does **not** change contract function signatures or the
      attestation schema — **or** the change is flagged here so
      `lafiya-web` can be updated in the same change set (see the
      README's Shared Contracts section).

## Notes for reviewers

Anything reviewers should look at first, or follow-ups deliberately left
out of scope.
