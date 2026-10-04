#!/usr/bin/env bash
# Configures stellar-cli for the local quickstart sidecar and funds a `dev`
# identity. Idempotent: safe to re-run.
set -euo pipefail

make wasm-contracts

echo "Waiting for local network at ${SOROBAN_HEALTH_URL}..."
for _ in $(seq 1 60); do
  curl -sf "${SOROBAN_HEALTH_URL}" >/dev/null && break
  sleep 5
done

stellar network add local \
  --rpc-url "${SOROBAN_RPC_URL}" \
  --network-passphrase "${SOROBAN_NETWORK_PASSPHRASE}" 2>/dev/null || true
stellar keys generate dev --network local 2>/dev/null || true
stellar keys fund dev --network local

cat <<'MSG'

Lafiya dev environment ready.
  make check             fmt, clippy, tests, contract wasm
  make test-integration  deploy + exercise contracts on the local network
  stellar keys address dev   funded `dev` identity on network `local`
MSG
