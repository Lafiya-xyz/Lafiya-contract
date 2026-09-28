#!/usr/bin/env bash
# DEPRECATED: This script is a thin compatibility wrapper.
# Use the Rust CLI instead:
#
#   lafiya-cli --network <name> attester <subcommand>
#   lafiya-cli --network <name> attestation <subcommand>
#   lafiya-cli --network <name> config show|list|env
#
# This wrapper will be removed in the next release.
set -euo pipefail
echo "DEPRECATION NOTICE: scripts/admin.sh is deprecated." >&2
echo "Use: lafiya-cli $*" >&2
echo "" >&2
exec lafiya-cli "$@"
