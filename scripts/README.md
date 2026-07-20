# Lafiya Scripts

This directory contains deployment and admin tooling that **all read from `config/networks.toml`** — no hardcoded RPC URLs or passphrases.

## Centralized Config

`config/networks.toml` is the single source of truth:

```toml
[testnet]
rpc_url = "https://soroban-testnet.stellar.org"
network_passphrase = "Test SDF Network ; September 2015"

[testnet.contracts]
attester_registry = "C..."
attestation_registry = "C..."
```

**Secrets policy:** Private keys, mnemonics, deployer secrets are **never** stored in `networks.toml`. They are managed via `stellar` CLI identities or env vars.

## Loader — Shared Between Deploy and Admin

**`scripts/lib/config.sh`** is the shared shell loader:

```bash
source ./scripts/lib/config.sh
load_network_config "testnet"        # reads config/networks.toml
echo $LAFIYA_RPC_URL
echo $LAFIYA_NETWORK_PASSPHRASE
```

Both `deploy.sh` and `admin.sh` source this file — zero duplication, zero hardcoded values.

**Rust loader:** `crates/lafiya-config` does the same for Rust tooling:

```rust
use lafiya_config::{load_networks, get_network};
let nets = load_networks(None)?;
let cfg = get_network(&nets, "testnet")?;
```

`crates/lafiya-cli` uses this Rust loader — so shell and Rust stacks share identical config.

## Switching Networks — One Flag

```bash
./scripts/deploy.sh --network testnet
./scripts/deploy.sh --network futurenet
./scripts/deploy.sh --network local
./scripts/deploy.sh --network mainnet

./scripts/admin.sh --network testnet config show
./scripts/admin.sh --network testnet attester is GABC...
./scripts/admin.sh --network testnet --source admin attester add GABC...

cargo run -p lafiya-cli -- --network testnet config show
cargo run -p lafiya-cli -- --network testnet attester is GABC...
cargo run -p lafiya-cli -- --network local config env
```

## Scripts

| Script | Purpose | Config Usage |
|--------|---------|--------------|
| `lib/config.sh` | Shared loader, parses TOML via python3 `tomllib`/`tomli`, exports `LAFIYA_*` vars | Source of truth |
| `deploy.sh` | Builds WASM and deploys both contracts via `stellar contract deploy`, then `initialize`, updates `networks.toml` | `--network` flag, no hardcoded RPC/passphrase |
| `admin.sh` | Bash admin CLI: attester allowlist mgmt, attestation queries | `--network` flag, same loader |
| `crates/lafiya-cli` | Rust admin CLI (preferred, more robust) | Uses `lafiya-config` crate reading same TOML |

### deploy.sh

```bash
./scripts/deploy.sh --network testnet --source deployer --admin GADMIN...
./scripts/deploy.sh --network local --dry-run
./scripts/deploy.sh --network testnet --build-only
```

- Builds `wasm32v1-none` artifacts
- Deploys via `stellar contract deploy --rpc-url $LAFIYA_RPC_URL --network-passphrase ...`
- Initializes with admin and links contracts
- Prompts to update `config/networks.toml` with new IDs

### admin.sh

```bash
./scripts/admin.sh --network testnet config show
./scripts/admin.sh --network testnet config list
./scripts/admin.sh --network testnet attester is G...
./scripts/admin.sh --network testnet --source admin attester add G...
./scripts/admin.sh --network testnet --source admin attester remove G...
./scripts/admin.sh --network testnet attestation get <64-hex>
```

### Rust CLI

```bash
cargo run -p lafiya-cli -- --network testnet config show
cargo run -p lafiya-cli -- config list
cargo run -p lafiya-cli -- --network testnet config env
cargo run -p lafiya-cli -- --network testnet attester is G...
```

Produces shell-friendly env output:

```bash
eval $(cargo run -p lafiya-cli -- --network testnet config env)
```

## Adding a New Network

Edit `config/networks.toml`:

```toml
[mytest]
rpc_url = "https://..."
network_passphrase = "..."

[mytest.contracts]
attester_registry = ""
attestation_registry = ""
```

No code changes needed — all tooling picks it up via `--network mytest`.

## CI

`cargo test -p lafiya-config` validates TOML parsing, missing network errors, and ensures no secret fields exist.

Makefile targets:

```bash
make config-check   # validates networks.toml, runs lafiya-config tests
make config-list    # lists networks
make deploy NETWORK=testnet
```
