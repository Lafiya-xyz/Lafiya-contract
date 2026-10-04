#!/bin/bash
# End-to-end dry run of reset detection against a local stellar/quickstart.
# Restarting the container wipes the ledger, which is exactly what an SDF
# testnet reset looks like to the detector.
#
# Usage: scripts/testnet_reset/dry-run-local.sh
# Requires: docker, stellar CLI, python3, jq. Modifies config/ only in a temp copy.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'docker rm -f lafiya-reset-dryrun >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

cp -r "$ROOT/config" "$ROOT/scripts" "$WORK/"
cd "$WORK"

start_quickstart() {
  docker rm -f lafiya-reset-dryrun >/dev/null 2>&1 || true
  docker run -d --name lafiya-reset-dryrun -p 8000:8000 stellar/quickstart --local --enable rpc >/dev/null
  until curl -sf -X POST -H 'Content-Type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' http://localhost:8000/soroban/rpc | grep -q healthy; do
    sleep 5
  done
}

echo "== 1. start quickstart and deploy"
start_quickstart
stellar network add dryrun --rpc-url http://localhost:8000/soroban/rpc \
  --network-passphrase "Standalone Network ; February 2017" 2>/dev/null || true
stellar keys generate dryrun-admin --network dryrun --fund --overwrite
(cd "$ROOT" && ./scripts/deploy-testnet.sh --identity dryrun-admin --network dryrun --yes)
python3 scripts/testnet_reset/reconcile.py "$ROOT/deployments/dryrun.json" --network local
sleep 30  # let the ledger advance past the baseline
python3 scripts/testnet_reset/detect.py --network local --update-state
python3 scripts/testnet_reset/detect.py --network local | tee /dev/stderr | grep -q '"healthy"'

echo "== 2. restart quickstart (simulated reset)"
start_quickstart
python3 scripts/testnet_reset/detect.py --network local | tee /dev/stderr | grep -q '"network-reset"'

echo "Dry run passed: healthy -> network-reset detected."
