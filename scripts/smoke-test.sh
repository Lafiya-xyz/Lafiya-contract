#!/usr/bin/env bash

# Smoke test for Lafiya contract deployment
# Requires the following environment variables:
#   ATT_REGISTRY        - Attestation Registry contract ID
#   ATTESTER_REGISTRY   - Attester Registry contract ID
#   NETWORK_URL         - Horizon URL for the target testnet
#   ADMIN_SECRET        - Secret key of an admin account with permission to add/remove attesters

set -euo pipefail

if [[ -z "${ATT_REGISTRY}" || -z "${ATTESTER_REGISTRY}" || -z "${NETWORK_URL}" || -z "${ADMIN_SECRET}" ]]; then
  echo "Error: One or more required environment variables are missing."
  echo "Required: ATT_REGISTRY ATTESTER_REGISTRY NETWORK_URL ADMIN_SECRET"
  exit 1
fi

# Helper function to run stellar-cli commands
run_cli() {
  local cmd="$1"
  echo "Running: ${cmd}"
  eval "stellar-cli ${cmd}"
}

# Generate a throwaway attester keypair
ATT_KEYPAIR=$(stellar-cli keypair generate)
ATT_ADDRESS=$(echo "$ATT_KEYPAIR" | grep "Address:" | awk '{print $2}')
ATT_SECRET=$(echo "$ATT_KEYPAIR" | grep "Secret:" | awk '{print $2}')

# Add temporary attester
run_cli "address add --address $ATT_ADDRESS --secret $ADMIN_SECRET --network $NETWORK_URL"
run_cli "contract invoke $ATTESTER_REGISTRY add_attester $ATT_ADDRESS --secret $ADMIN_SECRET --network $NETWORK_URL"

# Submit a test attestation (using a zero hash)
DUMMY_HASH="0x0000000000000000000000000000000000000000000000000000000000000000"
run_cli "contract invoke $ATT_REGISTRY submit_attestation $DUMMY_HASH $ATT_ADDRESS --secret $ATT_SECRET --network $NETWORK_URL"

# Read back the attestation
RESULT=$(run_cli "contract invoke $ATT_REGISTRY get_attestation $DUMMY_HASH --network $NETWORK_URL")
# Simple verification: ensure the attester address appears in the result
if echo "$RESULT" | grep -q "$ATT_ADDRESS"; then
  echo "Attestation verified successfully."
else
  echo "Verification failed: attester address not found in result."
  exit 1
fi

# Clean up: remove temporary attester
run_cli "contract invoke $ATTESTER_REGISTRY remove_attester $ATT_ADDRESS --secret $ADMIN_SECRET --network $NETWORK_URL"

echo "Smoke test completed successfully."
exit 0
