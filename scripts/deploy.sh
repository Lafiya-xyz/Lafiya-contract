#!/usr/bin/env bash
# DEPRECATED: This script is a thin compatibility wrapper.
# Use the Rust CLI instead:
#
#   lafiya-cli deploy --network <name> [--source <identity>] [--admin <address>]
#
# This wrapper will be removed in the next release.
# See: docs/runbooks/contract-upgrade.md
set -euo pipefail
echo "DEPRECATION NOTICE: scripts/deploy.sh is deprecated." >&2
echo "Use: lafiya-cli deploy --network ${STELLAR_NETWORK:-testnet} $*" >&2
echo "" >&2
exec lafiya-cli deploy "$@"
