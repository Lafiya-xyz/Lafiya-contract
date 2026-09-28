#!/usr/bin/env bash
# DEPRECATED: This script is a thin compatibility wrapper.
# Use the Rust CLI instead:
#
#   lafiya-cli --network testnet deploy [--source <identity>] [--admin <address>]
#
# This wrapper will be removed in the next release.
set -euo pipefail
echo "DEPRECATION NOTICE: scripts/deploy-testnet.sh is deprecated." >&2
echo "Use: lafiya-cli --network testnet deploy $*" >&2
echo "" >&2
exec lafiya-cli --network testnet deploy "$@"
