# Publishing Strategy for TypeScript Bindings

We generate TypeScript client bindings for the `attester-registry` and `attestation-registry`
contracts (see [`docs/typescript-bindings.md`](docs/typescript-bindings.md) for how they're
generated). This document is the canonical source for how those bindings are distributed to
consumers (`lafiya-web`, `lafiya-verifier`); README.md's "Publishing & Consumption" section and
`docs/typescript-bindings.md` should stay consistent with what's decided here.

## Decision

**Primary, in effect today:** the generated client code in `bindings/` is committed directly to
this repository. Consumers depend on it via:
- A git path dependency in the consuming project's `package.json` (e.g.
  `"@lafiya/contracts": "git+https://github.com/Lafiya-xyz/Lafiya-contract.git#semver:^0.1.0"`).
- A git submodule in the consuming repo.
- Standard workspace/monorepo references, if a monorepo setup is adopted later.

This keeps contract and binding changes version-locked in one place and requires no publishing
infrastructure to get started.

**Secondary, planned:** publish `bindings/attester-registry` and `bindings/attestation-registry`
to the `@lafiya` npm organization via a GitHub Action triggered off release tags, so consumers
who prefer a normal `npm install` can do so instead of a git dependency. This is intended to run
off the same tag that [ADR-0009's release manifest](docs/adr/0009-release-manifest-and-compatibility.md)
is generated from, so a published binding version and its `generated_from_wasm_sha256` never
disagree with the contract it wraps.

npm publishing is **not live yet**. It requires follow-up work that is out of scope for this
document alone:
- `bindings/attester-registry/package.json` and `bindings/attestation-registry/package.json` are
  currently named plainly (`attester-registry`, `attestation-registry`) with no `@lafiya` scope
  and no `publishConfig`. They need both before `npm publish --access public` would work as
  described here.
- Both packages are pinned at `"version": "0.0.0"`. Per ADR-0009's follow-up items, real
  versioning should start with the first release cut under that ADR, not before.
- No CI workflow exists yet to run `npm publish` on tag push.

## Rationale

- **No new infrastructure required now**: committing bindings and consuming them via git means
  `lafiya-web` can start using them today without this repo standing up npm publishing first.
- **Version-locked by construction**: a git dependency pinned to a tag/commit always matches the
  contract source it was generated from; there's no separate registry version to drift out of
  sync.
- **npm as an upgrade, not a replacement**: once the follow-up work above lands, npm publishing
  is additive — consumers who want a normal registry dependency can switch, while the committed
  `bindings/` directory keeps working for anyone using a git dependency or submodule.

## Coordination

- `lafiya-web` (or any other consumer) picks one of the git-based methods above today.
- When npm publishing goes live, the maintainer of `lafiya-web` can switch to depending on
  `@lafiya/attester-registry` / `@lafiya/attestation-registry` (or a combined package, if that's
  what's implemented) with the appropriate version range.
- Future contract changes trigger a bindings regeneration (`make bindings`); consumers on a git
  dependency update their lockfile, and consumers on npm (once available) bump their pinned
  version.
