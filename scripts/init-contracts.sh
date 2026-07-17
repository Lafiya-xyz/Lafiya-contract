#!/bin/bash
set -e

if [ ! -f .deployed_contracts ]; then
  echo "Error: .deployed_contracts not found. Run make deploy-testnet first."
  exit 1
fi

source .deployed_contracts

# Ensure we have a default identity
if ! stellar keys address default >/dev/null 2>&1; then
  echo "Default identity not found. Generating a new one..."
  stellar keys generate default --network testnet
fi

ADMIN=$(stellar keys address default)

echo "Initializing attester-registry ($ATTESTER_REGISTRY_ID) with admin $ADMIN..."
stellar contract invoke \
  --id $ATTESTER_REGISTRY_ID \
  --source default \
  --network testnet \
  -- \
  initialize \
  --admin $ADMIN

echo "Initializing attestation-registry ($ATTESTATION_REGISTRY_ID) with admin $ADMIN and attester_registry $ATTESTER_REGISTRY_ID..."
stellar contract invoke \
  --id $ATTESTATION_REGISTRY_ID \
  --source default \
  --network testnet \
  -- \
  initialize \
  --admin $ADMIN \
  --attester_registry $ATTESTER_REGISTRY_ID

echo "Contracts initialized successfully."
