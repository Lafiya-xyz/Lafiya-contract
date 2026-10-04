#!/bin/bash
# Seed demo fixture data (one demo attester and one demo attestation) into a
# freshly deployed system for integration environments.
#
# Usage: scripts/testnet_reset/seed-fixtures.sh <deployments/NETWORK.json> <admin-identity> [network]
set -euo pipefail

DEPLOYMENT="${1:?deployment json required}"
ADMIN="${2:?admin stellar-cli identity required}"
NETWORK="${3:-testnet}"

ATTESTER_REGISTRY=$(jq -r .attester_registry "$DEPLOYMENT")
ATTESTATION_REGISTRY=$(jq -r .attestation_registry "$DEPLOYMENT")

stellar keys generate demo-attester --network "$NETWORK" --fund --overwrite
DEMO_ATTESTER=$(stellar keys address demo-attester)

stellar contract invoke --id "$ATTESTER_REGISTRY" --source-account "$ADMIN" --network "$NETWORK" \
    -- add_attester --attester "$DEMO_ATTESTER"

# Fixed, obviously-synthetic record hash so consumers can recognise demo data.
DEMO_HASH="$(printf 'de%.0s' {1..32})"
stellar contract invoke --id "$ATTESTATION_REGISTRY" --source-account demo-attester --network "$NETWORK" \
    -- attest --attester "$DEMO_ATTESTER" --record_hash "$DEMO_HASH"

echo "Seeded demo attester $DEMO_ATTESTER and attestation $DEMO_HASH"
