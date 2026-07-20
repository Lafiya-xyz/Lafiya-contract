#!/bin/bash

# Exit immediately if a command exits with a non-zero status
set -e

# Lafiya Smart Contract Deployment Script
# Deploys both contracts to Stellar Testnet (or other networks) and initializes them.

NETWORK="testnet"
IDENTITY=""
ADMIN_ADDRESS=""
YES=false

# Help message
show_help() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  -i, --identity <identity>       stellar-cli identity name to use for deployment (or set STELLAR_IDENTITY)"
    echo "  -a, --admin-address <address>   Admin address for contract initialization (defaults to the identity's address)"
    echo "  -n, --network <network>         Stellar network (default: testnet)"
    echo "  -y, --yes                       Skip interactive confirmation prompt"
    echo "  -h, --help                      Show this help message"
    echo ""
    echo "Example:"
    echo "  $0 --identity my-testnet-account"
}

# Parse command line options
while [[ $# -gt 0 ]]; do
    case "$1" in
        -i|--identity)
            IDENTITY="$2"
            shift 2
            ;;
        -a|--admin-address)
            ADMIN_ADDRESS="$2"
            shift 2
            ;;
        -n|--network)
            NETWORK="$2"
            shift 2
            ;;
        -y|--yes)
            YES=true
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "Error: Unknown option: $1" >&2
            show_help >&2
            exit 1
            ;;
    esac
done

# Check if stellar CLI is installed
if ! command -v stellar &> /dev/null; then
    echo "Error: 'stellar' CLI is not installed or not in PATH." >&2
    echo "Please install it following instructions at: https://developers.stellar.org/docs/smart-contracts/getting-started/setup" >&2
    exit 1
fi

# Fallback to environment variable for identity if not provided via CLI
if [ -z "$IDENTITY" ]; then
    IDENTITY="$STELLAR_IDENTITY"
fi

if [ -z "$IDENTITY" ]; then
    echo "Error: Identity is required. Specify it with -i/--identity or the STELLAR_IDENTITY environment variable." >&2
    show_help >&2
    exit 1
fi

# Resolve admin address if not provided
if [ -z "$ADMIN_ADDRESS" ]; then
    echo "Resolving admin address for identity '$IDENTITY'..."
    if ! ADMIN_ADDRESS=$(stellar keys address "$IDENTITY" 2>/dev/null); then
        echo "Error: Failed to resolve address for identity '$IDENTITY' using 'stellar keys address'." >&2
        echo "Ensure the identity exists in stellar-cli, or provide the admin address explicitly using -a/--admin-address." >&2
        exit 1
    fi
fi

# Print configuration and ask for confirmation
echo "=========================================================="
echo "           Lafiya Soroban Contract Deployment"
echo "=========================================================="
echo "Network:       $NETWORK"
echo "Identity:      $IDENTITY"
echo "Admin Address: $ADMIN_ADDRESS"
echo "=========================================================="
echo ""

if [ "$YES" = false ]; then
    read -p "Do you want to proceed with the deployment to $NETWORK? (y/N): " CONFIRM
    if [[ ! "$CONFIRM" =~ ^[yY](es)?$ ]]; then
        echo "Deployment aborted."
        exit 0
    fi
fi

# 1. Build WASM contracts
echo "Building WASM contracts for release..."
cargo build --workspace --release --target wasm32v1-none

# Paths to the compiled WASM files
ATTESTER_WASM="target/wasm32v1-none/release/attester_registry.wasm"
ATTESTATION_WASM="target/wasm32v1-none/release/attestation_registry.wasm"

# Verify built files exist
if [ ! -f "$ATTESTER_WASM" ]; then
    echo "Error: $ATTESTER_WASM not found after build." >&2
    exit 1
fi
if [ ! -f "$ATTESTATION_WASM" ]; then
    echo "Error: $ATTESTATION_WASM not found after build." >&2
    exit 1
fi

# 2. Deploy attester-registry
echo "Deploying attester-registry..."
ATTESTER_REGISTRY_ID=$(stellar contract deploy \
    --wasm "$ATTESTER_WASM" \
    --source-account "$IDENTITY" \
    --network "$NETWORK")

if [ -z "$ATTESTER_REGISTRY_ID" ]; then
    echo "Error: Failed to deploy attester-registry." >&2
    exit 1
fi
echo "attester-registry deployed successfully. ID: $ATTESTER_REGISTRY_ID"
echo ""

# 3. Deploy attestation-registry
echo "Deploying attestation-registry..."
ATTESTATION_REGISTRY_ID=$(stellar contract deploy \
    --wasm "$ATTESTATION_WASM" \
    --source-account "$IDENTITY" \
    --network "$NETWORK")

if [ -z "$ATTESTATION_REGISTRY_ID" ]; then
    echo "Error: Failed to deploy attestation-registry." >&2
    exit 1
fi
echo "attestation-registry deployed successfully. ID: $ATTESTATION_REGISTRY_ID"
echo ""

# 4. Initialize attester-registry
echo "Initializing attester-registry..."
stellar contract invoke \
    --id "$ATTESTER_REGISTRY_ID" \
    --source-account "$IDENTITY" \
    --network "$NETWORK" \
    -- initialize \
    --admin "$ADMIN_ADDRESS"

echo "attester-registry initialized successfully."
echo ""

# 5. Initialize attestation-registry
echo "Initializing attestation-registry..."
stellar contract invoke \
    --id "$ATTESTATION_REGISTRY_ID" \
    --source-account "$IDENTITY" \
    --network "$NETWORK" \
    -- initialize \
    --admin "$ADMIN_ADDRESS" \
    --attester_registry "$ATTESTER_REGISTRY_ID"

echo "attestation-registry initialized successfully."
echo ""

# 6. Save deployment details to deployments/<network>.json
mkdir -p deployments
DEPLOYMENTS_FILE="deployments/${NETWORK}.json"

cat <<EOF > "$DEPLOYMENTS_FILE"
{
  "network": "$NETWORK",
  "attester_registry": "$ATTESTER_REGISTRY_ID",
  "attestation_registry": "$ATTESTATION_REGISTRY_ID",
  "admin": "$ADMIN_ADDRESS",
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

echo "=========================================================="
echo "                 Deployment Complete!"
echo "=========================================================="
echo "Contract IDs and configuration saved to: $DEPLOYMENTS_FILE"
echo "=========================================================="
