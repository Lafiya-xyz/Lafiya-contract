#!/usr/bin/env bash
set -euo pipefail

# Soroban Smart Contract Integration Test Suite
# Exercises deployment, contract initialization, allowlist management, and attestation flow on a local network node.

echo "========================================================"
echo "  Starting Lafiya Soroban Integration Test Suite"
echo "========================================================"

# 1. Locate CLI binary (supports 'stellar' or legacy 'soroban' executable)
if command -v stellar &> /dev/null; then
    CLI="stellar"
elif command -v soroban &> /dev/null; then
    CLI="soroban"
else
    echo "Error: Neither 'stellar' nor 'soroban' CLI is found in PATH."
    echo "Install stellar-cli: cargo install --locked stellar-cli or via install script."
    exit 1
fi

echo "Using CLI binary: $CLI ($($CLI --version | head -n 1))"

# 2. Check local network health
RPC_URL="${SOROBAN_RPC_URL:-http://localhost:8000/soroban/rpc}"
NETWORK_PASSPHRASE="${SOROBAN_NETWORK_PASSPHRASE:-Local Testing Network ; July 2022}"
HEALTH_URL="${SOROBAN_HEALTH_URL:-http://localhost:8000/health}"

echo "Checking local Soroban network health at $HEALTH_URL..."
MAX_RETRIES=60
COUNT=0
HEALTHY=false

while [ $COUNT -lt $MAX_RETRIES ]; do
    if curl -s -f "$HEALTH_URL" > /dev/null 2>&1 || curl -s -f "$RPC_URL" > /dev/null 2>&1; then
        HEALTHY=true
        break
    fi
    COUNT=$((COUNT + 1))
    echo "Waiting for local network node... ($COUNT/$MAX_RETRIES)"
    sleep 1
done

if [ "$HEALTHY" = false ]; then
    echo "Error: Local Soroban network node at $RPC_URL is unreachable."
    echo "Ensure local container is running: docker run -d -p 8000:8000 --name stellar stellar/quickstart:testing --local"
    exit 1
fi
echo "Local Soroban network is online and healthy!"

# 3. Configure network profile
echo "Configuring network 'local'..."
$CLI network add --global local \
    --rpc-url "$RPC_URL" \
    --network-passphrase "$NETWORK_PASSPHRASE" 2>/dev/null || true

# 4. Ensure release WASM contracts exist
ATTESTER_WASM="target/wasm32v1-none/release/attester_registry.wasm"
ATTESTATION_WASM="target/wasm32v1-none/release/attestation_registry.wasm"

if [ ! -f "$ATTESTER_WASM" ] || [ ! -f "$ATTESTATION_WASM" ]; then
    echo "WASM artifacts missing. Building workspace target wasm32v1-none..."
    cargo build --workspace --release --target wasm32v1-none
fi

# 5. Generate and fund test identities
echo "Generating test identities (admin and attester)..."
$CLI keys generate admin --global --network local 2>/dev/null || true
$CLI keys generate attester --global --network local 2>/dev/null || true

ADMIN_ADDR=$($CLI keys address admin)
ATTESTER_ADDR=$($CLI keys address attester)

echo "Admin address:    $ADMIN_ADDR"
echo "Attester address: $ATTESTER_ADDR"

echo "Funding test identities via Friendbot..."
$CLI keys fund admin --network local 2>/dev/null || true
$CLI keys fund attester --network local 2>/dev/null || true

# 6. Deploy contracts
echo "Deploying attester-registry..."
ATTESTER_REG_ID=$($CLI contract deploy \
    --wasm "$ATTESTER_WASM" \
    --source admin \
    --network local)
echo "Attester Registry deployed at: $ATTESTER_REG_ID"

echo "Deploying attestation-registry..."
ATTESTATION_REG_ID=$($CLI contract deploy \
    --wasm "$ATTESTATION_WASM" \
    --source admin \
    --network local)
echo "Attestation Registry deployed at: $ATTESTATION_REG_ID"

# 7. Initialize contracts
echo "Initializing attester-registry (admin: $ADMIN_ADDR)..."
$CLI contract invoke \
    --id "$ATTESTER_REG_ID" \
    --source admin \
    --network local \
    -- initialize --admin "$ADMIN_ADDR"

echo "Initializing attestation-registry (admin: $ADMIN_ADDR, registry: $ATTESTER_REG_ID)..."
$CLI contract invoke \
    --id "$ATTESTATION_REG_ID" \
    --source admin \
    --network local \
    -- initialize --admin "$ADMIN_ADDR" --attester_registry "$ATTESTER_REG_ID"

# 8. Test authorization gate (non-allowlisted attester attempt must fail)
RECORD_HASH="0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
echo "Testing attest from non-allowlisted attester (expected to be rejected)..."
if $CLI contract invoke \
    --id "$ATTESTATION_REG_ID" \
    --source attester \
    --network local \
    -- attest --attester "$ATTESTER_ADDR" --record_hash "$RECORD_HASH" >/dev/null 2>&1; then
    echo "ERROR: Attestation by non-allowlisted attester succeeded unexpectedly!"
    exit 1
else
    echo "PASSED: Non-allowlisted attester call rejected as expected."
fi

# 9. Allowlist attester in attester-registry
echo "Allowlisting attester..."
$CLI contract invoke \
    --id "$ATTESTER_REG_ID" \
    --source admin \
    --network local \
    -- add_attester --attester "$ATTESTER_ADDR"

echo "Verifying is_attester status..."
IS_ATTESTER_RES=$($CLI contract invoke \
    --id "$ATTESTER_REG_ID" \
    --source attester \
    --network local \
    -- is_attester --attester "$ATTESTER_ADDR")

echo "is_attester response: $IS_ATTESTER_RES"
if ! echo "$IS_ATTESTER_RES" | grep -i "true" >/dev/null; then
    echo "ERROR: is_attester check did not return true!"
    exit 1
fi
echo "PASSED: Attester successfully allowlisted."

# 10. Execute attest flow with allowlisted attester
echo "Submitting attestation for record hash ($RECORD_HASH)..."
ATTEST_RES=$($CLI contract invoke \
    --id "$ATTESTATION_REG_ID" \
    --source attester \
    --network local \
    -- attest --attester "$ATTESTER_ADDR" --record_hash "$RECORD_HASH")

echo "Attest output: $ATTEST_RES"

# 11. Retrieve and verify recorded attestation
echo "Querying get_attestation..."
GET_RES=$($CLI contract invoke \
    --id "$ATTESTATION_REG_ID" \
    --source attester \
    --network local \
    -- get_attestation --record_hash "$RECORD_HASH")

echo "get_attestation output: $GET_RES"

if echo "$GET_RES" | grep -q "$ATTESTER_ADDR"; then
    echo "PASSED: Retrieved attestation matches attester address $ATTESTER_ADDR."
else
    echo "ERROR: Retrieved attestation does not contain attester address $ATTESTER_ADDR!"
    exit 1
fi

echo "========================================================"
echo "  Integration Test Suite Completed Successfully! 🎉"
echo "========================================================"
