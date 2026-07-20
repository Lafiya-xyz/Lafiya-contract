#!/usr/bin/env bash
# Lafiya - Admin CLI (bash version)
# Provides attester allowlist management and attestation queries,
# using config/networks.toml for network resolution.
#
# Usage:
#   ./scripts/admin.sh --network testnet <command> [args]
#
# Commands:
#   config show              Show current network config
#   config list              List available networks
#   attester is <address>    Check if address is allowlisted
#   attester add <address>   Add attester (requires admin auth)
#   attester remove <address> Remove attester
#   attester list            (attempt) list attesters - note: registry doesn't enumerate, this shows placeholder
#   attestation get <64-char-hex-hash>  Lookup attestation by record hash hex
#   attest <record_hash_hex> --attester <address>   (client-side helper, requires attester auth)
#
# All network params come from config/networks.toml -- switching is --network flag.
# Secrets via stellar identities / env vars, never via config file.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/config.sh"

NETWORK="testnet"
CONFIG_PATH="$REPO_ROOT/config/networks.toml"
SOURCE_ACCOUNT="${STELLAR_ACCOUNT:-}"

usage() {
    cat <<EOF
Lafiya Admin CLI (bash)

Usage: $0 --network <name> <command> [args] [options]

Global Options:
  --network <name>    Network (local, standalone, testnet, futurenet, mainnet) Default: testnet
  --config <path>     Path to networks.toml (default: config/networks.toml)
  --source <acct>     Stellar identity for admin auth (or STELLAR_ACCOUNT env)
  -h, --help          Help

Commands:
  config show         Show resolved config for --network
  config list         List available networks
  attester is <addr>  Check if address is allowlisted (calls contract)
  attester add <addr> Add attester to allowlist (admin auth required)
  attester remove <addr> Remove attester
  attestation get <hex64>  Get attestation for record hash (hex 32-byte)
  
Examples:
  $0 --network testnet config show
  $0 --network testnet attester is GABC...
  $0 --network testnet --source admin attester add GABC...
  $0 --network local attestation get 0f8a...

Config file ($CONFIG_PATH) never holds secrets, only public RPC/passphrase/IDs.
Switch networks with one flag: --network testnet|futurenet|mainnet|local
EOF
}

# Parse global opts first
ARGS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --network)
            NETWORK="$2"
            shift 2
            ;;
        --config)
            CONFIG_PATH="$2"
            shift 2
            ;;
        --source)
            SOURCE_ACCOUNT="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            ARGS+=("$@")
            break
            ;;
        -*)
            echo "Unknown global option: $1" >&2
            usage >&2
            exit 1
            ;;
        *)
            ARGS+=("$1")
            shift
            ;;
    esac
done

set -- "${ARGS[@]:-}"

COMMAND="${1:-}"
SUBCOMMAND="${2:-}"
ARG1="${3:-}"

if [[ -z "$COMMAND" ]]; then
    usage
    exit 1
fi

# For config list, we don't need full load
if [[ "$COMMAND" == "config" && "$SUBCOMMAND" == "list" ]]; then
    echo "Available networks in $CONFIG_PATH:"
    list_networks "$CONFIG_PATH"
    exit 0
fi

# Load network config for all other commands
echo "==> Using network: $NETWORK (config: $CONFIG_PATH)" >&2
load_network_config "$NETWORK" "$CONFIG_PATH"

check_stellar_cli() {
    if ! command -v stellar >/dev/null 2>&1; then
        echo "ERROR: stellar CLI not found. Install: cargo install --locked stellar-cli" >&2
        exit 1
    fi
}

require_contract_id() {
    local kind="$1"
    local id="$2"
    if [[ -z "$id" ]]; then
        echo "ERROR: $kind contract ID not set for network '$NETWORK' in $CONFIG_PATH" >&2
        echo "Deploy first: ./scripts/deploy.sh --network $NETWORK" >&2
        exit 1
    fi
}

stellar_source_args=()
if [[ -n "$SOURCE_ACCOUNT" ]]; then
    stellar_source_args=(--source "$SOURCE_ACCOUNT")
fi

case "$COMMAND" in
    config)
        case "$SUBCOMMAND" in
            show)
                print_network_config "$NETWORK" "$CONFIG_PATH"
                ;;
            *)
                echo "Unknown config subcommand: $SUBCOMMAND" >&2
                usage >&2
                exit 1
                ;;
        esac
        ;;

    attester)
        require_contract_id "attester-registry" "$LAFIYA_ATTESTER_REGISTRY_ID"
        check_stellar_cli
        case "$SUBCOMMAND" in
            is)
                if [[ -z "$ARG1" ]]; then
                    echo "Usage: $0 --network $NETWORK attester is <address>" >&2
                    exit 1
                fi
                echo "==> Checking is_attester for $ARG1 on $LAFIYA_ATTESTER_REGISTRY_ID"
                stellar contract invoke \
                    --id "$LAFIYA_ATTESTER_REGISTRY_ID" \
                    --rpc-url "$LAFIYA_RPC_URL" \
                    --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
                    -- is_attester --attester "$ARG1"
                ;;
            add)
                if [[ -z "$ARG1" ]]; then
                    echo "Usage: $0 --network $NETWORK --source admin attester add <address>" >&2
                    exit 1
                fi
                echo "==> Adding attester $ARG1 (admin auth via ${SOURCE_ACCOUNT:-default})"
                stellar contract invoke \
                    --id "$LAFIYA_ATTESTER_REGISTRY_ID" \
                    --rpc-url "$LAFIYA_RPC_URL" \
                    --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
                    "${stellar_source_args[@]}" \
                    -- add_attester --attester "$ARG1"
                ;;
            remove)
                if [[ -z "$ARG1" ]]; then
                    echo "Usage: $0 --network $NETWORK --source admin attester remove <address>" >&2
                    exit 1
                fi
                echo "==> Removing attester $ARG1"
                stellar contract invoke \
                    --id "$LAFIYA_ATTESTER_REGISTRY_ID" \
                    --rpc-url "$LAFIYA_RPC_URL" \
                    --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
                    "${stellar_source_args[@]}" \
                    -- remove_attester --attester "$ARG1"
                ;;
            list)
                echo "Note: attester-registry does not support enumeration on-chain." >&2
                echo "You need to track attesters off-chain or via events." >&2
                echo "Contract ID: $LAFIYA_ATTESTER_REGISTRY_ID"
                echo "Use 'stellar contract invoke ... is_attester' to check individually."
                ;;
            *)
                echo "Unknown attester subcommand: $SUBCOMMAND" >&2
                usage >&2
                exit 1
                ;;
        esac
        ;;

    attestation)
        require_contract_id "attestation-registry" "$LAFIYA_ATTESTATION_REGISTRY_ID"
        check_stellar_cli
        case "$SUBCOMMAND" in
            get)
                if [[ -z "$ARG1" ]]; then
                    echo "Usage: $0 --network $NETWORK attestation get <hex_hash_64chars>" >&2
                    exit 1
                fi
                # ARG1 is hex string for BytesN<32>
                echo "==> Getting attestation for hash $ARG1 on $LAFIYA_ATTESTATION_REGISTRY_ID"
                stellar contract invoke \
                    --id "$LAFIYA_ATTESTATION_REGISTRY_ID" \
                    --rpc-url "$LAFIYA_RPC_URL" \
                    --network-passphrase "$LAFIYA_NETWORK_PASSPHRASE" \
                    -- get_attestation --record_hash "$ARG1"
                ;;
            *)
                echo "Unknown attestation subcommand: $SUBCOMMAND" >&2
                usage >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "Unknown command: $COMMAND" >&2
        usage >&2
        exit 1
        ;;
esac
