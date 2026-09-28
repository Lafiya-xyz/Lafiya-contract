#!/usr/bin/env bash
# DEPRECATED: This script is a thin compatibility wrapper.
# Use the Rust CLI instead:
#
#   lafiya-cli upgrade --contract <name> --id <C...> --source <identity> [options]
#
# This wrapper will be removed in the next release.
# See: docs/runbooks/contract-upgrade.md
set -euo pipefail
echo "DEPRECATION NOTICE: scripts/upgrade.sh is deprecated." >&2
echo "Use: lafiya-cli upgrade $*" >&2
echo "" >&2
exec lafiya-cli upgrade "$@"
