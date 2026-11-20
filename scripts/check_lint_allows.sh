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

# Fail if governance primitives are redefined outside `lafiya-contract-common`.
# Governance primitives (two-step admin, pause flags, TTL constants) must live in
# the common crate so that semantics, errors, and events stay converged.
common_crate='crates/lafiya-contract-common'

# 1) TTL constants must not be redefined per contract.
ttl_consts='INSTANCE_BUMP_AMOUNT|INSTANCE_LIFETIME_THRESHOLD'
bad_ttl=$(grep -rnE "const[[:space:]]+($ttl_consts)[[:space:]]*:" contracts crates --include='*.rs' \
  | grep -v "^${common_crate}/" || true)
if [[ -n "$bad_ttl" ]]; then
  echo "governance TTL constants redefined outside ${common_crate}:" >&2
  echo "$bad_ttl" >&2
  exit 1
fi

# 2) Two-step admin primitives must not be reimplemented per contract.
admin_fns='propose_admin|accept_admin'
bad_admin=$(grep -rnE "fn[[:space:]]+($admin_fns)[[:space:]]*\(" contracts crates --include='*.rs' \
  | grep -v "^${common_crate}/" || true)
if [[ -n "$bad_admin" ]]; then
  echo "two-step admin reimplemented outside ${common_crate}:" >&2
  echo "$bad_admin" >&2
  exit 1
fi

# 3) Pause flags must not be redefined per contract.
pause_flags='paused|is_paused'
bad_pause=$(grep -rnE "(const|static)[[:space:]]+($pause_flags)[[:space:]]*:|fn[[:space:]]+(pause|unpause|is_paused)[[:space:]]*\(" contracts crates --include='*.rs' \
  | grep -v "^${common_crate}/" || true)
if [[ -n "$bad_pause" ]]; then
  echo "pause primitives redefined outside ${common_crate}:" >&2
  echo "$bad_pause" >&2
  exit 1
fi