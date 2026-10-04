# Release Process

This document outlines the versioning, changelog, and release workflow for the Lafiya smart contracts.

---

## Versioning Policy

Lafiya smart contracts follow [Semantic Versioning (SemVer)](https://semver.org/).
- **Major (`X.0.0`)**: Released for backward-incompatible changes, such as breaking public API signature updates, storage schema changes that require state migration, or major architectural shifts.
- **Minor (`0.Y.0`)**: Released for backward-compatible features, such as adding new optional functions, events, or helper modules.
- **Patch (`0.0.Z`)**: Released for backward-compatible bug fixes, internal optimizations, or documentation updates.

### When to Cut a Version
A new version should be cut whenever:
1. A milestone (e.g., M1 Attestation, M2 Incentives) is reached and unit tests pass.
2. A feature branch containing public API changes or storage schema adjustments is merged into `main`.
3. An audit remediation or critical fix is ready for testnet/mainnet.

---

## Automated Release Flow

Releases are cut by automation; humans approve at the gates marked **(gate)**.

1. **Conventional Commits on `main`.** PR titles / squash messages use
   `feat:`, `fix:`, `feat!:` / `BREAKING CHANGE:` etc. A commit that changes
   `DataKey` or any `#[contracttype]` must end with a trailer:
   ```text
   Schema-Impact: additive -- new DataKey::Paused, absent reads as false
   ```
   (`git commit --amend --trailer "Schema-Impact: ..."`). The PR template has a
   required *Schema Impact* field as a reminder.
2. **Release PR.** `.github/workflows/release-please.yml` runs
   [release-please](https://github.com/googleapis/release-please) on every push to
   `main` and keeps one `chore(main): release X.Y.Z` PR open, containing the next
   SemVer bump — `[workspace.package] version` in `Cargo.toml`, the workspace
   entries in `Cargo.lock`, both `bindings/*/package.json` — and the generated
   `CHANGELOG.md` section (config: `release-please-config.json`,
   `.release-please-manifest.json`). Don't edit `CHANGELOG.md` by hand in
   feature PRs.
   - `.github/workflows/schema-impact.yml` fails the release PR if any commit since
     the last tag touches the storage schema without a `Schema-Impact:` trailer
     (`scripts/check_schema_impact.py`, tested by `scripts/test_check_schema_impact.py`).
   - **(gate)** A maintainer reviews the release PR, edits the CHANGELOG section to
     add the release's schema-impact statement (the trailers are listed in the
     release notes) and any migration steps, and merges it.
3. **Tag pipeline.** Merging tags `vX.Y.Z` and runs
   `.github/workflows/release-manifest.yml`:
   reproducible wasm build → `SHA256SUMS` → provenance-metadata assertion →
   `generate_release_manifest.py` → `validate_release_manifest.py` → manifest
   hashes checked against `SHA256SUMS` → bindings built and packed → signed SLSA
   provenance (`actions/attest-build-provenance`) for each `.wasm` and
   `release-manifest.json` → GitHub Release with the wasm, `SHA256SUMS`, manifest,
   provenance bundle, bindings tarballs, and notes → a follow-up PR copying the
   wasm into the upgrade-compatibility fixtures (`tests/fixtures/upgrade-compat/vX.Y.Z/`).
   A tag pushed by hand runs the same pipeline.
   - npm publishing of the bindings stays off until [PUBLISHING.md](../PUBLISHING.md)'s
     prerequisites are met; the tarballs are attached to the release instead.
   - **(gate)** Merge the fixtures PR.
4. **(gate) Deploy / upgrade** by following the
   [Contract Upgrade Runbook](runbooks/contract-upgrade.md), using the wasm hashes
   from the release's `SHA256SUMS`.

### Dry run

Actions → *Release* → *Run workflow* on any branch (or fork), with `dry_run`
checked (the default). It builds the full artifact set — wasm, `SHA256SUMS`,
manifest, bindings tarballs, `RELEASE_NOTES.md` — and uploads it as the
`release-<ref>` workflow artifact, without attesting, creating a release, or
opening the fixtures PR.

---

## Reproducible Builds

Anyone can rebuild the exact released bytes from the tagged source:

- `rust-toolchain.toml` pins an **exact** toolchain (not `stable`); every workflow
  installs it with `rustup toolchain install`. Dependabot's `rust-toolchain` rule
  proposes bumps; when merging one, update `REPRODUCIBLE_IMAGE` in the `Makefile`
  to the matching `rust:<version>-slim` digest in the same PR.
- `make wasm-reproducible` builds in `rust:<version>-slim` **pinned by digest**, with
  the checkout mounted at `/lafiya`, and writes
  `target/wasm32v1-none/release/SHA256SUMS`. Inside, it runs
  `cargo xtask wasm --reproducible`, which also works natively: it remaps the
  workspace and `CARGO_HOME` paths (`--remap-path-prefix`), sets
  `SOURCE_DATE_EPOCH` to the commit time, and injects `LAFIYA_GIT_COMMIT` (below).
  The release wasm is the unoptimized `cargo build` output, so `stellar-cli`
  isn't part of the build.
- CI (`wasm-reproducible` jobs in `ci.yml`) builds every commit twice — in the
  container and natively in a different checkout path on another runner — and
  fails unless the SHA-256 values are identical. The tag pipeline also checks that
  the manifest's `wasm.sha256` values equal the reproducible build's `SHA256SUMS`.

### Verifying a deployed contract

"Contract `C…` on network N runs release `vX.Y.Z`":

```bash
# 1. The wasm the contract actually runs (fetched by contract ID).
stellar contract fetch --id C... --network testnet --out-file onchain.wasm
sha256sum onchain.wasm

# 2. Rebuild the tag.
git clone https://github.com/Lafiya-xyz/Lafiya-contract && cd Lafiya-contract
git checkout vX.Y.Z
make wasm-reproducible          # or: cargo xtask wasm --reproducible

# 3. Compare: the on-chain hash must appear in the rebuilt SHA256SUMS,
#    and match the release's SHA256SUMS / release-manifest.json.
grep "$(sha256sum onchain.wasm | cut -d' ' -f1)" target/wasm32v1-none/release/SHA256SUMS

# 4. The wasm names its own source (SEP-46 metadata, see below).
stellar contract info meta --wasm onchain.wasm
```

---

## Source Provenance

### In-wasm metadata

Each contract's `src/lib.rs` embeds `contractmetav0` entries
([SEP-46](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0046.md))
using the keys from
[SEP-55 — Contract Build Verification](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0055.md),
which explorers such as Stellar Expert read to link a contract to its source:

| Key | Value |
|-----|-------|
| `source_repo` | `github:Lafiya-xyz/Lafiya-contract` |
| `home_domain` | `lafiya-xyz.github.io` |
| `crate_name`, `crate_version` | from the crate's `Cargo.toml` |
| `source_rev` | the git commit, from `LAFIYA_GIT_COMMIT` (set by `cargo xtask wasm --reproducible`; `unknown` in ordinary dev builds) |

The values depend only on the commit, so reproducibility holds. CI and the tag
pipeline assert them with `scripts/check_wasm_meta.py --commit <sha>`; locally,
`stellar contract info meta --wasm <file>` shows them.

### Signed build provenance

Tag builds attach signed [SLSA](https://slsa.dev/) provenance for every `.wasm`
and for `release-manifest.json` (also shipped as `provenance.intoto.jsonl` on the
release). To verify a release asset — or a wasm fetched from chain, which is the
same bytes:

```bash
gh attestation verify attester_registry.wasm --repo Lafiya-xyz/Lafiya-contract
gh attestation verify onchain.wasm --repo Lafiya-xyz/Lafiya-contract
```

A successful verification shows the file was built by this repository's release
workflow from the commit named in the attestation. Together with the on-chain
wasm hash (Verifying a deployed contract, above) that ties a deployed contract ID
to a reviewed tag.

---

## Release Manifest

Every release includes a **release manifest**: one JSON document binding the contract
wasm hashes, storage schema versions, generated TypeScript bindings, event schemas, and
per-network deployment state for that release into a single, machine-checkable record.
See [ADR-0010](adr/0010-release-manifest-and-compatibility.md) for the schema, the
compatibility policy it encodes, and how downstream repositories (`lafiya-web`,
`lafiya-verifier`) use it to check compatibility before pinning a release. Locally:

```bash
cargo xtask release-manifest
```

`.github/workflows/release-manifest.yml` is a prototype CI workflow that does this on
every `v*.*.*` tag; see ADR-0010 "Follow-up" for what remains before it is load-bearing.

## Interface Version and Runtime Negotiation

Every contract exposes `get_interface() -> InterfaceInfo` with its `contract_kind`,
`interface_version`, enabled `features`, `schema_version`, and `event_version`. The
values are constants in each `contracts/*/src/lib.rs` (`CONTRACT_KIND`,
`INTERFACE_VERSION`, `FEATURES`, `EVENT_VERSION`).

- **Bump `INTERFACE_VERSION`** on any breaking public ABI change (a removed or changed
  function, error, event, or type). Adding an entry is not breaking.
- `scripts/conformance/check_snapshot.py` records the version next to each snapshot in
  `snapshots/interface_versions.json`. `make conformance` fails if the constant and the
  recorded version disagree, and `make conformance-update` refuses a breaking snapshot
  change without a bump.
- Clients call `get_interface()` once per session and refuse to operate if the kind or
  version is wrong, rather than probing calls. For example:

  ```bash
  lafiya-cli --network testnet interface attester
  # error: contract CDX… is interface v1; this CLI requires >= v3
  ```

  A contract that predates `get_interface` is reported as interface v0. `contract_kind`
  is a weak identity check when wiring contracts, not proof of authenticity.

---

## Testnet & Mainnet Redeployment

Because Soroban smart contracts are immutable once deployed (unless an upgrade path is explicitly programmed), deploying a new version generally requires deploying new WASM bytecode and updating the contract addresses referenced by downstream consumers (such as the frontend app `lafiya-web`).

For details on the redeployment, initialization, and upgrade state migration processes, please cross-reference the upgrade runbook:
- [Contract Upgrade Runbook](runbooks/contract-upgrade.md)

Always follow the instructions in the runbook when performing redeployments to ensure that downstream services are not interrupted.
