#!/bin/bash

# Exit immediately if a command exits with a non-zero status
set -e

# Lafiya Smart Contract Deployment Script
# Deploys both contracts with their security-sensitive constructor arguments.
#
# The multisig-account contract is deployed with the policy-based constructor
# (`__constructor_policy`), which takes a `SignerPolicy` instead of the legacy
# `Vec<BytesN<32>>` signer list. The legacy `set_signers` entry point is
# deprecated; new deployments must start in the policy model.

NETWORK="testnet"
IDENTITY=""
ADMIN_ADDRESS=""
SIGNER_POLICY=""
YES=false

# Help message
show_help() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  -i, --identity <identity>       stellar-cli identity name to use for deployment (or set STELLAR_IDENTITY)"
    echo "  -a, --admin-address <address>   Admin address for both contracts (defaults to the identity's address)"
    echo "  -p, --signer-policy <json>      SignerPolicy JSON for the multisig-account policy constructor"
    echo "                                  (default: single ed25519 signer = admin address, weight 1, threshold 1)"
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
        -p|--signer-policy)
            SIGNER_POLICY="$2"
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

# Default signer policy: a single ed25519 signer (the admin) with weight 1 and
# threshold 1. This mirrors the legacy single-signer model but goes through the
# policy constructor so new deployments never start in the deprecated model.
if [ -z "$SIGNER_POLICY" ]; then
    SIGNER_POLICY=$(cat <<EOF
{"signers":[{"Ed25519":{"key":"$ADMIN_ADDRESS","weight":1}}],"threshold":1}
EOF
)
fi

# Print configuration and ask for confirmation
echo "=========================================================="
echo "           Lafiya Soroban Contract Deployment"
echo "=========================================================="
echo "Network:       $NETWORK"
echo "Identity:      $IDENTITY"
echo "Admin Address: $ADMIN_ADDRESS"
echo "Signer Policy: $SIGNER_POLICY"
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
# Uses the policy-based constructor so the account starts with a SignerPolicy
# rather than the deprecated legacy signer list.
echo "Deploying attester-registry (policy constructor)..."
ATTESTER_REGISTRY_ID=$(stellar contract deploy \
    --wasm "$ATTESTER_WASM" \
    --source-account "$IDENTITY" \
    --network "$NETWORK" \
    -- --admin "$ADMIN_ADDRESS" --signer_policy "$SIGNER_POLICY")

if [ -z "$ATTESTER_REGISTRY_ID" ]; then
    echo "Error: Failed to deploy attester-registry." >&2
    exit 1
fi
echo "attester-registry deployed successfully. ID: $ATTESTER_REGISTRY_ID"
echo ""

# 3. Deploy attestation-registry
# Also uses the policy-based constructor for the multisig-account surface.
echo "Deploying attestation-registry (policy constructor)..."
ATTESTATION_REGISTRY_ID=$(stellar contract deploy \
    --wasm "$ATTESTATION_WASM" \
    --source-account "$IDENTITY" \
    --network "$NETWORK" \
    -- --admin "$ADMIN_ADDRESS" --attester_registry "$ATTESTER_REGISTRY_ID" --signer_policy "$SIGNER_POLICY")

if [ -z "$ATTESTATION_REGISTRY_ID" ]; then
    echo "Error: Failed to deploy attestation-registry." >&2
    exit 1
fi
echo "attestation-registry deployed successfully. ID: $ATTESTATION_REGISTRY_ID"
echo ""

# 4. Save deployment details to deployments/<network>.json
mkdir -p deployments
DEPLOYMENTS_FILE="deployments/${NETWORK}.json"

cat <<EOF > "$DEPLOYMENTS_FILE"
{
  "network": "$NETWORK",
  "attester_registry": "$ATTESTER_REGISTRY_ID",
  "attestation_registry": "$ATTESTATION_REGISTRY_ID",
  "admin": "$ADMIN_ADDRESS",
  "signer_policy": $SIGNER_POLICY,
  "constructor": "policy",
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

echo "=========================================================="
echo "                 Deployment Complete!"
echo "=========================================================="
echo "Contract IDs and configuration saved to: $DEPLOYMENTS_FILE"
echo "=========================================================="
