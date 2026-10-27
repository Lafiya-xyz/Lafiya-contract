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
off the same tag that [ADR-0010's release manifest](docs/adr/0010-release-manifest-and-compatibility.md)
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

## Prebuilt `lafiya-cli` Binaries

Operators who don't have a Rust toolchain can install a prebuilt `lafiya-cli` binary from the
GitHub Releases page. Because `lafiya-cli` holds signing capability, every release artifact is
accompanied by checksums, cosign signatures, and SLSA provenance, and the install script verifies
them before anything is executed.

### Release artifacts

Each release (cut by `.github/workflows/release-please.yml`) publishes, per target:

| Target | Artifact |
| --- | --- |
| Linux x86_64 (glibc) | `lafiya-cli-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 (musl, static) | `lafiya-cli-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux aarch64 (musl, static) | `lafiya-cli-<version>-aarch64-unknown-linux-musl.tar.gz` |
| macOS x86_64 | `lafiya-cli-<version>-x86_64-apple-darwin.tar.gz` |
| macOS aarch64 | `lafiya-cli-<version>-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `lafiya-cli-<version>-x86_64-pc-windows-msvc.zip` |

Alongside the archives, every release includes:

- `SHA256SUMS` — checksums for all archives.
- `SHA256SUMS.sig` and `SHA256SUMS.pem` — cosign keyless signature over `SHA256SUMS` and the
  signing certificate.
- `<artifact>.sig` and `<artifact>.pem` — per-artifact cosign signatures (defense in depth).
- `multiple.intoto.jsonl` — SLSA provenance attestation covering all artifacts, generated with
  `slsa-framework/slsa-github-generator`.

### Verifying install script (recommended)

The install script downloads the right artifact for your platform, verifies the checksum, the
cosign signature, and (when `cosign` and `slsa-verifier` are available) the SLSA provenance,
and only then installs the binary. It refuses to run if any verification step fails.

```sh
# Install the latest release into ~/.local/bin (override with --install-dir)
curl -fsSL https://raw.githubusercontent.com/Lafiya-xyz/Lafiya-contract/main/scripts/install-lafiya-cli.sh \
  | sh -s -- --verify

# Pin a specific version
curl -fsSL https://raw.githubusercontent.com/Lafiya-xyz/Lafiya-contract/main/scripts/install-lafiya-cli.sh \
  | sh -s -- --version v1.2.3 --verify
```

If you'd rather not pipe a script into a shell, download it first, inspect it, then run it:

```sh
curl -fsSLO https://raw.githubusercontent.com/Lafiya-xyz/Lafiya-contract/main/scripts/install-lafiya-cli.sh
less install-lafiya-cli.sh
sh install-lafiya-cli.sh --version v1.2.3 --verify
```

The script requires `cosign` (for signature verification) and `sha256sum`/`shasum`; it will
attempt to verify SLSA provenance with `slsa-verifier` if that tool is on `PATH`, and will fail
closed if `--verify` is passed and any required tool is missing.

### Manual verification

If you install an archive by hand, verify it before extracting:

1. **Checksum.** Download `SHA256SUMS` from the same release and check the archive:

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   # macOS: shasum -a 256 -c SHA256SUMS
   ```

2. **Cosign signature over the checksum file.** Verify the keyless signature against this
   repository's release workflow identity:

   ```sh
   cosign verify-blob \
     --certificate SHA256SUMS.pem \
     --signature SHA256SUMS.sig \
     --certificate-identity-regexp '^https://github.com/Lafiya-xyz/Lafiya-contract/.github/workflows/release-please.yml@refs/tags/' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com \
     SHA256SUMS
   ```

   A successful run prints `Verified OK`. If it doesn't, do not run the binary.

3. **SLSA provenance.** Confirm the artifact was built by this repository's release workflow:

   ```sh
   slsa-verifier verify-artifact \
     --provenance-path multiple.intoto.jsonl \
     --source-uri github.com/Lafiya-xyz/Lafiya-contract \
     --source-tag v1.2.3 \
     lafiya-cli-v1.2.3-x86_64-unknown-linux-musl.tar.gz
   ```

   The verifier checks the provenance's builder identity, source repository, and source tag
   against the artifact digest.

### Reproducibility check (Linux target)

The Linux musl target is built with a pinned toolchain and `--locked` dependencies so an
independent rebuild produces a byte-identical archive. To reproduce a release artifact:

```sh
git clone https://github.com/Lafiya-xyz/Lafiya-contract.git
cd Lafiya-contract
git checkout v1.2.3

# Build in the same container image the release workflow uses
docker run --rm -v "$PWD:/src" -w /src \
  ghcr.io/lafiya-xyz/lafiya-cli-build:1.2.3 \
  cargo build --release --locked --target x86_64-unknown-linux-musl

# Package exactly as the release workflow does
./scripts/package-lafiya-cli.sh x86_64-unknown-linux-musl v1.2.3

sha256sum lafiya-cli-v1.2.3-x86_64-unknown-linux-musl.tar.gz
```

The resulting checksum must match the entry in the release's `SHA256SUMS`. CI runs this same
reproducibility check on every release candidate (see the `reproducible-linux` job in
`.github/workflows/release-please.yml`); a mismatch fails the release.

### CI verification

The release workflow's `verify-install` job downloads the published artifacts, runs the install
script with `--verify` against them, and exercises the manual checksum/cosign/provenance steps
above, so the instructions in this document are tested on every release. The workflow runs with
least-privilege permissions: `contents: write` only on the job that creates the release, and
`id-token: write` only on the jobs that need keyless signing or provenance generation.
