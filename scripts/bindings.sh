#!/bin/bash
set -e

if [ ! -f .deployed_contracts ]; then
  echo "Error: .deployed_contracts not found. Run make deploy-testnet first."
  exit 1
fi

source .deployed_contracts

echo "Generating TypeScript bindings..."
mkdir -p bindings

stellar contract bindings typescript \
  --network testnet \
  --contract-id $ATTESTER_REGISTRY_ID \
  --output-dir bindings/attester-registry \
  --overwrite

stellar contract bindings typescript \
  --network testnet \
  --contract-id $ATTESTATION_REGISTRY_ID \
  --output-dir bindings/attestation-registry \
  --overwrite

echo "Bindings generated in bindings/ directory."
