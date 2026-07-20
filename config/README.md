# Network Configuration

This directory holds the canonical source of truth for Lafiya's Soroban network parameters and contract IDs.

## File

- `networks.toml` - per-network RPC endpoints, passphrases, and deployed contract IDs.

## Format

```toml
[testnet]
rpc_url = "https://soroban-testnet.stellar.org"
network_passphrase = "Test SDF Network ; September 2015"

[testnet.contracts]
attester_registry = "C..."
attestation_registry = "C..."
```

### Networks

| Name | RPC URL | Passphrase | Use |
|------|---------|------------|-----|
| `local` / `standalone` | `http://localhost:8000/soroban/rpc` | `Standalone Network ; February 2017` | Local dev via `stellar/quickstart` |
| `testnet` | `https://soroban-testnet.stellar.org` | `Test SDF Network ; September 2015` | Pre-alpha testnet deployment |
| `futurenet` | `https://rpc-futurenet.stellar.org` | `Test SDF Future Network ; October 2022` | Future protocol testing |
| `mainnet` | `https://mainnet.sorobanrpc.com` | `Public Global Stellar Network ; September 2015` | Production (post-audit) |

### Secrets policy

**Never** store in this file:
- Private keys
- Mnemonics / seed phrases
- Deployer secret keys
- API keys

This file is committed to git and must contain only public data. Secrets are passed via environment variables or `stellar` CLI identity management:

```bash
# Deployer identity is managed by stellar-cli, not this file
stellar keys generate deployer --network testnet
# or use env:
export STELLAR_DEPLOYER_SECRET_KEY="S..."
```

## Usage

### Switching networks — one flag

All tooling uses `--network <name>` which reads from this config:

```bash
./scripts/deploy.sh --network testnet
./scripts/admin.sh --network testnet list-attesters

cargo run -p lafiya-cli -- --network testnet config show
cargo run -p lafiya-cli -- --network testnet attester is C...GABC
```

### Loader — shared between deploy and admin

**Rust:**
```rust
use lafiya_config::{load_networks, get_network};

let networks = load_networks(None)?; // auto-discovers config/networks.toml
let testnet = get_network(&networks, "testnet")?;
println!("RPC: {}", testnet.rpc_url);
```

**Shell (bash):** `scripts/lib/config.sh` is the shared loader:

```bash
source ./scripts/lib/config.sh
load_network_config "testnet"
echo "$LAFIYA_RPC_URL"
echo "$LAFIYA_NETWORK_PASSPHRASE"
echo "$LAFIYA_ATTESTER_REGISTRY_ID"
```

The shell loader uses `python3` with `tomllib`/`tomli` to parse TOML robustly — no hardcoded values.

## Adding a new network

1. Add a new table in `networks.toml`:
   ```toml
   [mynewnet]
   rpc_url = "https://..."
   network_passphrase = "..."

   [mynewnet.contracts]
   attester_registry = ""
   attestation_registry = ""
   ```
2. No code change needed — `--network mynewnet` works everywhere.

## Updating after deploy

After `./scripts/deploy.sh --network testnet`, copy the output contract IDs into `networks.toml`:

```toml
[testnet.contracts]
attester_registry = "CA6P..."
attestation_registry = "CB2X..."
```

Commit the updated `networks.toml` — contract IDs are public.
