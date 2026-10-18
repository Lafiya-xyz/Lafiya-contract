#!/usr/bin/env bash
# Fail if a lafiya lint is allowed without a `reason = "..."`.
set -euo pipefail
lints='persistent_set_without_ttl|storage_in_loop|unchecked_amount_arithmetic'
bad=$(grep -rnE "allow\(($lints)\)|allow\(($lints)[^r]*\)" contracts crates --include='*.rs' || true)
if [[ -n "$bad" ]]; then
  echo "lafiya lint allowed without a reason:" >&2
  echo "$bad" >&2
  exit 1
fi
