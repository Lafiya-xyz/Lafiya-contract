#!/usr/bin/env bash

# Canary attestation: submit one domain-separated attestation from the
# dedicated canary attester and verify it reads back. See
# docs/runbooks/canary-attestations.md.
# Required env: ATT_REGISTRY NETWORK_URL CANARY_SECRET CANARY_ADDRESS NETWORK_NAME
# Optional env: MAX_LATENCY_SECS (default 60)

set -euo pipefail

for v in ATT_REGISTRY NETWORK_URL CANARY_SECRET CANARY_ADDRESS NETWORK_NAME; do
  if [[ -z "${!v:-}" ]]; then
    echo "Error: $v is required." >&2
    exit 1
  fi
done

if [[ "$NETWORK_NAME" == "mainnet" && "${CANARY_MAINNET_APPROVED:-}" != "true" ]]; then
  echo "Canary on mainnet is disabled until approved (CANARY_MAINNET_APPROVED=true)." >&2
  exit 0
fi

# Domain-separated hash: sha256("lafiya.canary.v1" | network | UTC hour).
HOUR=$(date -u +%Y%m%d%H)
HASH="0x$(printf 'lafiya.canary.v1|%s|%s' "$NETWORK_NAME" "$HOUR" | sha256sum | awk '{print $1}')"

START=$(date +%s)
stellar-cli contract invoke "$ATT_REGISTRY" submit_attestation "$HASH" "$CANARY_ADDRESS" \
  --secret "$CANARY_SECRET" --network "$NETWORK_URL"
RESULT=$(stellar-cli contract invoke "$ATT_REGISTRY" get_attestation "$HASH" --network "$NETWORK_URL")
ELAPSED=$(( $(date +%s) - START ))

if ! grep -q "$CANARY_ADDRESS" <<<"$RESULT"; then
  echo "Canary failed: canary attester not found in read-back." >&2
  exit 1
fi
if (( ELAPSED > ${MAX_LATENCY_SECS:-60} )); then
  echo "Canary slow: write+read took ${ELAPSED}s (limit ${MAX_LATENCY_SECS:-60}s)." >&2
  exit 1
fi
echo "Canary ok in ${ELAPSED}s ($HASH)."
