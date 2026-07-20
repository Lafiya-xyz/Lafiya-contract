#!/usr/bin/env bash
# Lafiya - Deployment Script
# Deploys attester-registry and attestation-registry to selected network,
# using config/networks.toml for RPC URL and passphrase.
#
# Usage:
#   ./scripts/deploy.sh --network testnet [--source <identity-or-secret-env>] [--admin <address>]
#
# Environment:
#   - No secrets stored in config/networks.toml.
#   - Deployer account managed via stellar CLI identities or env vars:
#       STELLAR_ACCOUNT (identity name) or use --source flag.
#   - Admin address defaults to deployer if not provided.
#
# Prerequisites:
#   - stellar CLI installed (https://developers.stellar.org/docs/tools/developer-tools)
#   - Deployer funded (friendbot for testnet/futurenet)
#   - Config present at config/networks.toml

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# shellcheck source=./lib/config.sh
source "$SCRIPT_DIR/lib/config.sh"

NETWORK="testnet"
SOURCE_ACCOUNT="${STELLAR_ACCOUNT:-}"
ADMIN_ADDRESS="${ADMIN_ADDRESS:-}"
CONFIG_PATH="$REPO_ROOT/config/networks.toml"
DRY_RUN="false"
BUILD_ONLY="false"

usage() {
    cat <<EOF
Lafiya Contract Deployment

Usage: $0 [options]

Options:
  --network <name>      Network to deploy to (local, standalone, testnet, futurenet, mainnet)
                        Reads RPC and passphrase from config/networks.toml
                        Default: testnet
  --source <account>    Stellar identity name or secret key alias (managed by stellar CLI)
                        Or set STELLAR_ACCOUNT env var
  --admin <address>     Admin address for contracts (defaults to source account address)
  --config <path>       Path to networks.toml (default: config/networks.toml)
  --dry-run             Show what would be done without deploying
  --build-only          Only build WASM, don't deploy
  -h, --help            Show this help

Examples:
  $0 --network testnet
  $0 --network testnet --source deployer
  $0 --network local --admin GABC...
  $0 --network testnet --dry-run

Config file ($CONFIG_PATH) holds only public data (RPC, passphrase, contract IDs).
Secrets must be provided via stellar identities or env vars.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --network)
            NETWORK="$2"
            shift 2
            ;;
        --source)
            SOURCE_ACCOUNT="$2"
            shift 2
            ;;
        --admin)
            ADMIN_ADDRESS="$2"
            shift 2
            ;;
        --config)
            CONFIG_PATH="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN="true"
            shift
            ;;
        --build-only)
            BUILD_ONLY="true"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

echo "==> Loading network config: $NETWORK from $CONFIG_PATH"
load_network_config "$NETWORK" "$CONFIG_PATH"

echo "    Network: $LAFIYA_NETWORK"
echo "    RPC URL: $LAFIYA_RPC_URL"
echo "    Passphrase: $LAFIYA_NETWORK_PASSPHRASE"
echo "    Existing attester-registry: ${LAFIYA_ATTESTER_REGISTRY_ID:-<none>}"
echo "    Existing attestation-registry: ${LAFIYA_ATTESTATION_REGISTRY_ID:-<none>}"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] Would deploy to $NETWORK"
fi

# Build WASM artifacts
echo "==> Building WASM artifacts..."
if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] cargo build --workspace --release --target wasm32v1-none"
else
    (cd "$REPO_ROOT" && cargo build --workspace --release --target wasm32v1-none)
fi

if [[ "$BUILD_ONLY" == "true" ]]; then
    echo "==> Build only complete."
    exit 0
fi

# Check stellar CLI (allow missing for dry-run)
if ! command -v stellar >/dev/null 2>&1; then
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "WARNING: stellar CLI not found, but continuing because --dry-run" >&2
    else
        echo "ERROR: stellar CLI not found. Install from https://developers.stellar.org/docs/tools/developer-tools" >&2
        echo "Alternatively use: cargo install --locked stellar-cli" >&2
        exit 1
    fi
fi

# Determine source arg for stellar CLI
STELLAR_SOURCE_ARGS=()
if [[ -n "$SOURCE_ACCOUNT" ]]; then
    STELLAR_SOURCE_ARGS=(--source "$SOURCE_ACCOUNT")
fi

# Resolve admin: if not supplied, use source identity address or require it
if [[ -z "$ADMIN_ADDRESS" ]]; then
    if [[ -n "$SOURCE_ACCOUNT" ]]; then
        echo "==> Resolving admin address from source '$SOURCE_ACCOUNT'..."
        if [[ "$DRY_RUN" != "true" ]]; then
            ADMIN_ADDRESS="$(stellar keys address "$SOURCE_ACCOUNT" 2>/dev/null || true)"
        fi
    fi
    if [[ -z "$ADMIN_ADDRESS" ]]; then
        echo "WARNING: --admin not provided. Contracts will need an admin address at initialization." >&2
        echo "Provide --admin G... or ensure --source resolves to an address." >&2
        # For dry-run, use placeholder
        if [[ "$DRY_RUN" == "true" ]]; then
            ADMIN_ADDRESS="GADMINADDRESSPLACEHOLDERFORDRYRUNXXXXXXXXXXXXXXXXXX"
        else
            echo "ERROR: Could not resolve admin address. Provide --admin" >&2
            exit 1
        fi
    fi
fi

echo "==> Deployer source: ${SOURCE_ACCOUNT:-<default>}"
echo "==> Admin address: $ADMIN_ADDRESS"

ATTESTER_WASM="$REPO_ROOT/target/wasm32v1-none/release/attester_registry.wasm"
ATTESTATION_WASM="$REPO_ROOT/target/wasm32v1-none/release/attestation_registry.wasm"

if [[ "$DRY_RUN" != "true" ]]; then
    if [[ ! -f "$ATTESTER_WASM" ]]; then
        echo "ERROR: WASM not found at $ATTESTER_WASM" >&2
        echo "Run cargo build --workspace --release --target wasm32v1-none or use --dry-run" >&2
        exit 1
    fi
    if [[ ! -f "$ATTESTATION_WASM" ]]; then
        echo "ERROR: WASM not found at $ATTESTATION_WASM" >&2
        exit 1
    fi
fi

deploy_contract() {
    local wasm_path="$1"
    local network="$2"
    local rpc_url="$3"
    local passphrase="$4"
    local label="$5"

    echo "==> Deploying $label..." >&2
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "[DRY RUN] stellar contract deploy \\" >&2
        echo "  --wasm $wasm_path \\" >&2
        echo "  --rpc-url $rpc_url \\" >&2
        echo "  --network-passphrase \"$passphrase\" ${STELLAR_SOURCE_ARGS[*]}" >&2
        echo "CPLACEHOLDER${label}XXX" # placeholder ID
        return
    fi

    stellar contract deploy \
        --wasm "$wasm_path" \
        --rpc-url "$rpc_url" \
        --network-passphrase "$passphrase" \
        "${STELLAR_SOURCE_ARGS[@]}"
}

initialize_contract() {
    local contract_id="$1"
    local func="$2"
    shift 2
    # remaining args are function args as passed

    echo "==> Initializing $contract_id :: $func ..." >&2
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "[DRY RUN] stellar contract invoke --id $contract_id -- $func $*" >&2
        return
    fi

    stellar contract invoke \
        --id "$contract_id" \
        --rpc-url "$LAFIYA_RPC_URL" \
        --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
        "${STELLAR_SOURCE_ARGS[@]}" \
        -- "$func" "$@"
}

# Deploy attester-registry
ATTESTER_ID="$(deploy_contract "$ATTESTER_WASM" "$NETWORK" "$LAFIYA_RPC_URL" "$LAFIYA_NETWORK_PASSPHRASE" "attester-registry")"
ATTESTER_ID="$(echo "$ATTESTER_ID" | tr -d '\n' | xargs)" # trim
echo "    attester-registry ID: $ATTESTER_ID"

# Deploy attestation-registry
ATTESTATION_ID="$(deploy_contract "$ATTESTATION_WASM" "$NETWORK" "$LAFIYA_RPC_URL" "$LAFIYA_NETWORK_PASSPHRASE" "attestation-registry")"
ATTESTATION_ID="$(echo "$ATTESTATION_ID" | tr -d '\n' | xargs)"
echo "    attestation-registry ID: $ATTESTATION_ID"

# Initialize contracts
echo "==> Initializing attester-registry with admin $ADMIN_ADDRESS"
if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] initialize --admin $ADMIN_ADDRESS on $ATTESTER_ID"
else
    stellar contract invoke \
        --id "$ATTESTER_ID" \
        --rpc-url "$LAFIYA_RPC_URL" \
        --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
        "${STELLAR_SOURCE_ARGS[@]}" \
        -- initialize --admin "$ADMIN_ADDRESS" || {
            echo "Note: initialize may have failed if already initialized (expected on re-deploy)" >&2
        }
fi

echo "==> Initializing attestation-registry with admin $ADMIN_ADDRESS and attester-registry $ATTESTER_ID"
if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] initialize --admin $ADMIN_ADDRESS --attester_registry $ATTESTER_ID on $ATTESTATION_ID"
else
    stellar contract invoke \
        --id "$ATTESTATION_ID" \
        --rpc-url "$LAFIYA_RPC_URL" \
        --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
        "${STELLAR_SOURCE_ARGS[@]}" \
        -- initialize --admin "$ADMIN_ADDRESS" --attester_registry "$ATTESTER_ID" || {
            echo "Note: initialize may have failed if already initialized" >&2
        }
fi

echo ""
echo "==> Deployment complete for network: $NETWORK"
echo "    attester_registry = \"$ATTESTER_ID\""
echo "    attestation_registry = \"$ATTESTATION_ID\""
echo ""
echo "Next steps:"
echo "  1. Update config/networks.toml:"
echo "     [$NETWORK.contracts]"
echo "     attester_registry = \"$ATTESTER_ID\""
echo "     attestation_registry = \"$ATTESTATION_ID\""
echo ""
echo "  2. Verify with admin CLI:"
echo "     ./scripts/admin.sh --network $NETWORK list-attesters"
echo "     cargo run -p lafiya-cli -- --network $NETWORK config show"

# Auto-update config file helper (optional, prints instruction but doesn't auto-edit unless --auto-update flag could be added)
# For convenience, we offer to update if not dry-run and python available
if [[ "$DRY_RUN" != "true" ]]; then
    echo ""
    read -rp "Update config/networks.toml automatically? [y/N]: " confirm
    if [[ "$confirm" =~ ^[Yy]$ ]]; then
        python3 - "$CONFIG_PATH" "$NETWORK" "$ATTESTER_ID" "$ATTESTATION_ID" <<'PY'
import sys
config_path, network, attester_id, attestation_id = sys.argv[1:5]
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
import pathlib, re
# Simple text replacement to preserve comments/formatting
text = pathlib.Path(config_path).read_text()

# Regex to replace inside [network.contracts] section
# We'll do a stateful rewrite: find the section header and replace the two keys.
def update_section(text, net, key, value):
    # Find [net.contracts] block
    pattern = rf'(\[{re.escape(net)}\.contracts\][^\[]*?{re.escape(key)}\s*=\s*)".*?"'
    repl = rf'\1"{value}"'
    new_text, count = re.subn(pattern, repl, text, flags=re.DOTALL)
    if count == 0:
        # If not found, try single quotes or empty
        pattern2 = rf'(\[{re.escape(net)}\.contracts\][^\[]*?{re.escape(key)}\s*=\s*).+'
        # fallback: insert
        print(f"Warning: could not find {key} pattern, manual edit needed")
        return text
    return new_text

text = update_section(text, network, "attester_registry", attester_id)
text = update_section(text, network, "attestation_registry", attestation_id)
pathlib.Path(config_path).write_text(text)
print(f"Updated {config_path}")
PY
    fi
fi
