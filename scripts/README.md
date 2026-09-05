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
| `deploy-testnet.sh` | Simpler deploy: relies on the `stellar` CLI's own configured networks (not `networks.toml`), writes results to `deployments/<network>.json` instead of updating the config | `--network`/`-n` flag, does **not** read `networks.toml` |
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

### deploy-testnet.sh vs deploy.sh

Both deploy `attester-registry` and `attestation-registry` and initialize them, but they are **not interchangeable**:

- **`deploy.sh`** (preferred) — reads RPC URL/passphrase from `config/networks.toml`, supports `--dry-run`/`--build-only`, and can auto-update `networks.toml` with the new contract IDs. Use this for any network already defined in `networks.toml`.
- **`deploy-testnet.sh`** — relies on the `stellar` CLI's own pre-configured network (via `stellar network add`), skips `networks.toml` entirely, and instead writes a `deployments/<network>.json` record. Use this only if you manage networks directly through the `stellar` CLI rather than `config/networks.toml`.

```bash
./scripts/deploy-testnet.sh --identity my-testnet-account
./scripts/deploy-testnet.sh --identity my-testnet-account --network futurenet -y
```

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

## Input Validation

Operator input is validated locally, before anything is handed to the `stellar` CLI,
so malformed values fail immediately instead of producing a late error from the network.

| Value | Rule |
| --- | --- |
| `--network` | 1-32 characters, letters/digits/`-`/`_`, and present in `networks.toml` |
| Attester address | 56-character `G...` account or `C...` contract strkey |
| Contract IDs (from config or from a deploy) | 56-character `C...` strkey |
| Record hash | 64 hex characters (32 bytes) |
| `--admin` | 56-character `G...` account address |
| `--source` | `stellar` identity name (letters/digits/`.`/`-`/`_`) or a `G...` address; secret keys are rejected |
| `rpc_url` | `http://` or `https://` with a host |

Additional guarantees:

- Partially deployed profiles (only one of the two registry IDs recorded) are reported
  explicitly instead of failing later with a confusing contract error.
- `deploy.sh` refuses to run without `--source`/`STELLAR_ACCOUNT` and without a resolvable
  admin address, unless `--dry-run` or `--build-only` is used.
- Error messages name the offending field and the expected shape; they never echo secrets.

Shared implementations:

- Shell: `scripts/lib/validate.sh`, sourced by `deploy.sh` and `admin.sh`.
  Run its offline self-test with `./scripts/lib/validate.sh --self-test`.
- Rust: `lafiya_config::validation`, which additionally verifies the strkey CRC16
  checksum, so a single mistyped character in an address is caught before any call.

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
