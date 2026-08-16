#!/usr/bin/env bash

# Smoke test for Lafiya contract deployment
# Requires the following environment variables:
#   ATT_REGISTRY        - Attestation Registry contract ID
#   ATTESTER_REGISTRY   - Attester Registry contract ID
#   NETWORK_URL         - Horizon URL for the target testnet
#   ADMIN_SECRET        - Secret key of an admin account with permission to add/remove attesters

set -euo pipefail

if [[ -z "${ATT_REGISTRY:-}" || -z "${ATTESTER_REGISTRY:-}" || -z "${NETWORK_URL:-}" || -z "${ADMIN_SECRET:-}" ]]; then
  echo "Error: One or more required environment variables are missing." >&2
  echo "Required: ATT_REGISTRY ATTESTER_REGISTRY NETWORK_URL ADMIN_SECRET" >&2
  exit 1
fi

# Helper function to run stellar-cli commands safely without eval
# Arguments are passed directly to prevent shell injection.
# Sensitive arguments (e.g. secrets) are redacted from logs.
run_cli() {
  local -a cmd=("$@")
  local -a display_cmd=()
  local i=0

  while [[ "$i" -lt "${#cmd[@]}" ]]; do
    if [[ "${cmd[$i]}" == "--secret" ]]; then
      display_cmd+=("${cmd[$i]}")
      display_cmd+=("[REDACTED]")
      i=$((i + 2))
    else
      display_cmd+=("${cmd[$i]}")
      i=$((i + 1))
    fi
  done

  echo "Running: stellar-cli ${display_cmd[*]}"
  stellar-cli "${cmd[@]}"
}

# Generate a throwaway attester keypair
ATT_KEYPAIR=$(stellar-cli keypair generate)
ATT_ADDRESS=$(echo "$ATT_KEYPAIR" | grep "Address:" | awk '{print $2}')
ATT_SECRET=$(echo "$ATT_KEYPAIR" | grep "Secret:" | awk '{print $2}')

if [[ -z "${ATT_ADDRESS:-}" || -z "${ATT_SECRET:-}" ]]; then
  echo "Error: Failed to generate temporary attester keypair." >&2
  exit 1
fi

# Add temporary attester
run_cli address add --address "$ATT_ADDRESS" --secret "$ADMIN_SECRET" --network "$NETWORK_URL"
run_cli contract invoke "$ATTESTER_REGISTRY" add_attester "$ATT_ADDRESS" --secret "$ADMIN_SECRET" --network "$NETWORK_URL"

# Submit a test attestation (using a zero hash)
DUMMY_HASH="0x0000000000000000000000000000000000000000000000000000000000000000"
run_cli contract invoke "$ATT_REGISTRY" submit_attestation "$DUMMY_HASH" "$ATT_ADDRESS" --secret "$ATT_SECRET" --network "$NETWORK_URL"

# Read back the attestation
RESULT=$(run_cli contract invoke "$ATT_REGISTRY" get_attestation "$DUMMY_HASH" --network "$NETWORK_URL")
# Simple verification: ensure the attester address appears in the result
if echo "$RESULT" | grep -q "$ATT_ADDRESS"; then
  echo "Attestation verified successfully."
else
  echo "Verification failed: attester address not found in result." >&2
  exit 1
fi

# Clean up: remove temporary attester
run_cli contract invoke "$ATTESTER_REGISTRY" remove_attester "$ATT_ADDRESS" --secret "$ADMIN_SECRET" --network "$NETWORK_URL"

echo "Smoke test completed successfully."
exit 0
