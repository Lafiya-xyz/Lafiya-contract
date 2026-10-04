#!/bin/bash
# Append one row to the testnet deployment ledger and push it to the
# `testnet-ledger` branch, so CI deployments never write to main directly.
#
# Usage: scripts/append-deployment-ledger.sh "<markdown table row>"
set -euo pipefail

ROW="${1:?usage: $0 \"<markdown table row>\"}"
LEDGER="docs/deployments/testnet.md"
BRANCH="testnet-ledger"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

if git ls-remote --exit-code --heads origin "$BRANCH" > /dev/null; then
  git fetch origin "$BRANCH"
  git checkout -B "$BRANCH" "origin/$BRANCH"
else
  git checkout -B "$BRANCH"
fi

echo "$ROW" >> "$LEDGER"
git add "$LEDGER"
git commit -m "chore(ledger): record testnet deployment"
git push origin "$BRANCH"
