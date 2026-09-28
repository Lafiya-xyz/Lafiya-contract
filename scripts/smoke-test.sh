#!/usr/bin/env bash
# DEPRECATED: This script is a thin compatibility wrapper.
# Use the Rust CLI instead:
#
#   lafiya-cli --network <name> smoke-test [--source <identity>]
#
# This wrapper will be removed in the next release.
set -euo pipefail
echo "DEPRECATION NOTICE: scripts/smoke-test.sh is deprecated." >&2
echo "Use: lafiya-cli smoke-test $*" >&2
echo "" >&2
exec lafiya-cli smoke-test "$@"
