# Reproducible Environment

## One command

```bash
make audit-env              # build, lint, test, fuzz, budget + coverage reports
make audit-env DEPLOY=1     # ...and deploy to a local Stellar network
```

Both run [`scripts/audit-env.sh`](../scripts/audit-env.sh), which writes
its reports to `target/audit/`:

| File | Contents |
|---|---|
| `commit.txt`, `toolchain.txt` | Exact commit and toolchain used |
| `wasm-sha256.txt` | SHA-256 of each built contract Wasm (compare with the release manifest) |
| `test.txt` | `cargo test --workspace` output |
| `fuzz.txt` | Property tests, `PROPTEST_CASES=1024` by default (override with the env var) |
| `budget.txt` | CPU and memory budget checkpoints (`make bench`, see [docs/storage-cost.md](../docs/storage-cost.md)) |
| `coverage.txt` | Line coverage summary (`cargo llvm-cov`) |
| `local-deploy.txt` | Local-network deploy and end-to-end flow (`tests/integration/run.sh`), with `DEPLOY=1` |

## Setup

**Option A: devcontainer (recommended).** Open the repository in VS Code
or GitHub Codespaces and choose "Reopen in Container". The configuration
in [`.devcontainer/devcontainer.json`](../.devcontainer/devcontainer.json)
installs Rust (with `wasm32v1-none`), Docker, Python 3.12, `stellar-cli`,
and `cargo-llvm-cov`. Then run `make audit-env`.

**Option B: local machine.**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # toolchain pinned by rust-toolchain.toml
cargo install --locked stellar-cli cargo-llvm-cov
# Docker is needed only for DEPLOY=1 (stellar container start local)
make audit-env
```

## Other useful commands

```bash
make conformance                     # interface snapshots, error/event docs, bindings drift
cargo test -p attestation-registry   # a single contract
PROPTEST_CASES=10000 cargo test fuzz_test   # longer fuzz run
```
