#!/usr/bin/env bash
# Lightweight, dependency-free regression test for scripts/smoke-test.sh.
#
# Verifies, without any real Stellar CLI or network access, that:
#   - stellar-cli is invoked via literal argument arrays (never eval), so
#     shell metacharacters/spaces embedded in secrets or URLs cannot alter
#     the command that actually executes.
#   - Secret values (ADMIN_SECRET) are never written to the script's logs.
#
# Usage: ./scripts/smoke-test.test.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SMOKE_TEST="$SCRIPT_DIR/smoke-test.sh"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

MOCK_BIN_DIR="$WORK_DIR/bin"
mkdir -p "$MOCK_BIN_DIR"
ARGV_LOG="$WORK_DIR/argv.log"
CANARY_FILE="$WORK_DIR/should-not-exist"

# Mock `stellar-cli`: records every argument it receives (one per call, one
# line each) and returns canned output so smoke-test.sh can run its full
# flow with no network access.
cat > "$MOCK_BIN_DIR/stellar-cli" <<'MOCK'
#!/usr/bin/env bash
{
  echo "--- call ---"
  for arg in "$@"; do
    printf 'ARG<%s>\n' "$arg"
  done
} >> "$ARGV_LOG"

if [[ "$1" == "keypair" ]]; then
  echo "Address: GTESTADDRESS1234"
  echo "Secret: SMOCKATTESTERSECRET"
elif [[ "$1" == "contract" && "${4:-}" == "get_attestation" ]]; then
  echo "attester: GTESTADDRESS1234 attested=true"
else
  echo "OK"
fi
MOCK
chmod +x "$MOCK_BIN_DIR/stellar-cli"

# Secret containing whitespace and shell metacharacters. Under the old
# eval-based implementation this would either break argument boundaries or
# execute the embedded command substitution. The $(...) below is intentionally
# literal (single-quoted) -- it must NOT expand here.
# shellcheck disable=SC2016
INJECTION_SECRET='sup3r secret; $(touch '"$CANARY_FILE"') && echo pwned'

set +e
OUTPUT="$(
  ARGV_LOG="$ARGV_LOG" \
  PATH="$MOCK_BIN_DIR:$PATH" \
  ATT_REGISTRY="CATTREGISTRY" \
  ATTESTER_REGISTRY="CATTESTERREGISTRY" \
  NETWORK_URL="https://horizon-testnet.example.org" \
  ADMIN_SECRET="$INJECTION_SECRET" \
  bash "$SMOKE_TEST" 2>&1
)"
STATUS=$?
set -e

fail() {
  echo "FAIL: $1" >&2
  echo "--- captured output ---" >&2
  echo "$OUTPUT" >&2
  exit 1
}

[[ "$STATUS" -eq 0 ]] || fail "smoke-test.sh exited with status $STATUS"

[[ "$OUTPUT" == *"Smoke test completed successfully."* ]] || fail "script did not report success"

[[ -e "$CANARY_FILE" ]] && fail "injected secret was executed (eval-style command injection)"

grep -qF "$INJECTION_SECRET" "$ARGV_LOG" || fail "secret was not passed to stellar-cli as a literal argument"

echo "$OUTPUT" | grep -qF "$INJECTION_SECRET" && fail "secret value was printed in script output/logs"

echo "$OUTPUT" | grep -qF -- '--secret ****' || fail "expected redacted '--secret ****' in logs"

echo "PASS: smoke-test.sh avoids eval, preserves argument boundaries, and redacts secrets"
