#!/usr/bin/env bash
# End-to-end test: deploy the registries to a local quickstart, drive a
# scenario, index it, and compare the indexer API with contract state.
#
# Prerequisites:
#   docker run -d -p 8000:8000 --name stellar stellar/quickstart:testing --local
#   docker run -d -p 5432:5432 -e POSTGRES_PASSWORD=pg postgres:16-alpine
#   stellar CLI on PATH
#
# Usage (from the repository root):
#   DATABASE_URL=postgres://postgres:pg@localhost:5432/postgres \
#     crates/lafiya-indexer/tests/e2e.sh
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL}"
RPC_URL="${SOROBAN_RPC_URL:-http://localhost:8000/soroban/rpc}"
PASSPHRASE="${SOROBAN_NETWORK_PASSPHRASE:-Standalone Network ; February 2017}"
NET=(--rpc-url "$RPC_URL" --network-passphrase "$PASSPHRASE")

rpc() {
    curl -sf -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\"}"
}
until rpc getHealth | jq -e '.result.status == "healthy"' >/dev/null; do
    echo "waiting for RPC at $RPC_URL..."
    sleep 2
done

cargo build --release --locked --target wasm32v1-none -p attester-registry -p attestation-registry

for key in e2e-admin e2e-a e2e-b e2e-c; do
    stellar keys generate "$key" --overwrite >/dev/null 2>&1
    # Friendbot starts after RPC reports healthy.
    until stellar keys fund "$key" "${NET[@]}" >/dev/null 2>&1; do
        echo "waiting for friendbot..."
        sleep 3
    done
done
ADMIN=$(stellar keys address e2e-admin)
A=$(stellar keys address e2e-a)
B=$(stellar keys address e2e-b)
C=$(stellar keys address e2e-c)

START_LEDGER=$(rpc getLatestLedger | jq -r .result.sequence)

deploy() {
    stellar contract deploy --wasm "target/wasm32v1-none/release/$1.wasm" \
        --source e2e-admin "${NET[@]}"
}
invoke() {
    local id=$1 source=$2
    shift 2
    stellar contract invoke --id "$id" --source "$source" "${NET[@]}" -- "$@" >/dev/null
}

ATTESTERS=$(deploy attester_registry)
ATTESTATIONS=$(deploy attestation_registry)
invoke "$ATTESTERS" e2e-admin initialize --admin "$ADMIN"
invoke "$ATTESTATIONS" e2e-admin initialize --admin "$ADMIN" --attester_registry "$ATTESTERS"

R1=$(printf '11%.0s' {1..32})
R2=$(printf '22%.0s' {1..32})
R3=$(printf '33%.0s' {1..32})

# A: active with a region; B: suspended; C: removed.
invoke "$ATTESTERS" e2e-admin add_attester --attester "$A"
invoke "$ATTESTERS" e2e-admin add_attester --attester "$B"
invoke "$ATTESTERS" e2e-admin add_attester --attester "$C"
invoke "$ATTESTERS" e2e-admin update_attester_info --attester "$A" --region '"lagos"'
# R1 attested by A; R2 attested by B then revoked; R3 attested by C then
# superseded by A.
invoke "$ATTESTATIONS" e2e-a attest --attester "$A" --record_hash "$R1"
invoke "$ATTESTATIONS" e2e-b attest --attester "$B" --record_hash "$R2"
invoke "$ATTESTATIONS" e2e-admin revoke_attestation --record_hash "$R2"
invoke "$ATTESTATIONS" e2e-c attest --attester "$C" --record_hash "$R3"
invoke "$ATTESTATIONS" e2e-a attest --attester "$A" --record_hash "$R3"
invoke "$ATTESTERS" e2e-admin suspend_attester --attester "$B"
invoke "$ATTESTERS" e2e-admin remove_attester --attester "$C"

LAFIYA_E2E_RPC_URL="$RPC_URL" \
LAFIYA_E2E_START_LEDGER="$START_LEDGER" \
LAFIYA_E2E_ATTESTER_REGISTRY="$ATTESTERS" \
LAFIYA_E2E_ATTESTATION_REGISTRY="$ATTESTATIONS" \
    cargo test -p lafiya-indexer --test e2e -- --nocapture
