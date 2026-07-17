#!/bin/bash
set -e

echo "Deploying contracts to testnet..."

# Make sure wasm files exist, build them if not
if [ ! -f target/wasm32v1-none/release/attester_registry.wasm ]; then
    make wasm
fi

# Ensure we have a default identity
if ! stellar keys address default >/dev/null 2>&1; then
  echo "Default identity not found. Generating a new one..."
  stellar keys generate default --network testnet
fi

echo "Deploying attester-registry..."
ATTESTER_REGISTRY_ID=$(stellar contract deploy \
  --wasm target/wasm32v1-none/release/attester_registry.wasm \
  --source default \
  --network testnet)
echo "attester-registry deployed: $ATTESTER_REGISTRY_ID"

echo "Deploying attestation-registry..."
ATTESTATION_REGISTRY_ID=$(stellar contract deploy \
  --wasm target/wasm32v1-none/release/attestation_registry.wasm \
  --source default \
  --network testnet)
echo "attestation-registry deployed: $ATTESTATION_REGISTRY_ID"

# Save IDs to a file for init-contracts.sh
cat <<EOF > .deployed_contracts
ATTESTER_REGISTRY_ID=$ATTESTER_REGISTRY_ID
ATTESTATION_REGISTRY_ID=$ATTESTATION_REGISTRY_ID
EOF

echo "Deployment complete. Contract IDs saved to .deployed_contracts."
