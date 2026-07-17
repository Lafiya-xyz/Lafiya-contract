#!/usr/bin/env bash
#
# upgrade.sh — perform an on-chain upgrade of a Lafiya Soroban contract.
#
# Automates the mechanical steps of docs/runbooks/contract-upgrade.md:
#
#   1. build the release wasm (unless --wasm / --skip-build)
#   2. enforce the wasm-size budget (network cap: 65,536 bytes)
#   3. compute the wasm hash locally (sha256)
#   4. upload the wasm to the ledger (--optimize=false, byte-identical) and
#      verify the network-reported hash matches the local hash
#   5. submit the admin-authorized `upgrade(new_wasm_hash)` call
#   6. optionally submit `migrate()` (schema-changing upgrades, incl. the
#      first upgrade of a legacy pre-versioning instance)
#   7. verify post-upgrade: `get_schema_version` == --expected-schema-version,
#      and the code on-chain (fetch + sha256) matches the uploaded hash
#
# The pre-upgrade *human* steps of the runbook (clean tag, tests green,
# changelog entry, reviewer hash reproduction) must still be done by hand.
#
# Usage: scripts/upgrade.sh --contract attester-registry --id C... --source ALICE [options]
#
# Options:
#   --contract NAME              attester-registry | attestation-registry (required)
#   --id CONTRACT_ID             deployed contract id, C...                 (required, or CONTRACT_ID)
#   --source IDENTITY_OR_SECRET  admin signer known to stellar-cli          (required, or SOURCE_ACCOUNT)
#   --network NAME               stellar-cli network [default: testnet, or STELLAR_NETWORK]
#   --wasm PATH                  prebuilt wasm (skips build)
#   --skip-build                 use the existing release artifact in target/
#   --run-migrate                invoke migrate() after the upgrade (schema-changing
#                                releases / legacy bootstrap; requires --expected-schema-version)
#   --expected-schema-version N  assert get_schema_version == N post-upgrade
#   --max-wasm-bytes N           size budget [default: 65536, or MAX_WASM_BYTES]
#   --dry-run                    print every command without submitting anything
#   -h, --help                   this help
#
# Environment honored: RPC endpoints via STELLAR_RPC_URL (stellar-cli native).
set -euo pipefail

# ---------------------------------------------------------------- flags ----
CONTRACT=""
CONTRACT_ID="${CONTRACT_ID:-}"
SOURCE_ACCOUNT="${SOURCE_ACCOUNT:-}"
NETWORK="${STELLAR_NETWORK:-testnet}"
WASM_PATH=""
SKIP_BUILD=0
DRY_RUN=0
RUN_MIGRATE=0
EXPECTED_SCHEMA_VERSION="${EXPECTED_SCHEMA_VERSION:-}"
MAX_WASM_BYTES="${MAX_WASM_BYTES:-65536}"

die() { echo "ERROR: $*" >&2; exit 1; }
say() { echo "==> $*"; }

usage() { sed -n '2,45p' "${BASH_SOURCE[0]}" | sed 's/^#\{0,1\} \{0,1\}//'; }

while [ $# -gt 0 ]; do
    case "$1" in
        --contract)  CONTRACT="$2"; shift 2 ;;
        --id)        CONTRACT_ID="$2"; shift 2 ;;
        --source)    SOURCE_ACCOUNT="$2"; shift 2 ;;
        --network)   NETWORK="$2"; shift 2 ;;
        --wasm)      WASM_PATH="$2"; SKIP_BUILD=1; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --run-migrate) RUN_MIGRATE=1; shift ;;
        --expected-schema-version) EXPECTED_SCHEMA_VERSION="$2"; shift 2 ;;
        --max-wasm-bytes) MAX_WASM_BYTES="$2"; shift 2 ;;
        --dry-run)   DRY_RUN=1; shift ;;
        -h|--help)   usage; exit 0 ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

# ------------------------------------------------------------- validate ----
case "$CONTRACT" in
    attester-registry|attestation-registry) ;;
    *) die "--contract must be attester-registry or attestation-registry" ;;
esac
[ -n "$CONTRACT_ID" ]    || die "missing --id CONTRACT_ID (or CONTRACT_ID env)"
[ -n "$SOURCE_ACCOUNT" ] || die "missing --source ADMIN_IDENTITY (or SOURCE_ACCOUNT env)"
if [ "$RUN_MIGRATE" -eq 1 ] && [ -z "$EXPECTED_SCHEMA_VERSION" ]; then
    die "--run-migrate requires --expected-schema-version (ambiguity is unacceptable here)"
fi

command -v stellar >/dev/null 2>&1 || die "stellar-cli not found in PATH (https://developers.stellar.org/docs/tools/cli)"
if command -v sha256sum >/dev/null 2>&1; then SHA256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then SHA256() { shasum -a 256 "$1" | awk '{print $1}'; }
else die "need sha256sum or shasum"; fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
WASM_FILE="${CONTRACT//-/_}.wasm"

say "contract:            $CONTRACT"
say "contract id:         $CONTRACT_ID"
say "network:             $NETWORK"
say "source (admin):      $SOURCE_ACCOUNT"
say "run migrate:         $([ "$RUN_MIGRATE" -eq 1 ] && echo yes || echo no)"
say "expected schema ver: ${EXPECTED_SCHEMA_VERSION:-<not asserted>}"
[ "$DRY_RUN" -eq 1 ] && say "MODE:                DRY RUN (nothing will be submitted)"

# ------------------------------------------------------- 1. build (opt) ----
if [ "$SKIP_BUILD" -eq 0 ]; then
    say "step 1/7: building $CONTRACT (release, --locked, wasm32v1-none)"
    (cd "$REPO_ROOT" && cargo build --release --locked --target wasm32v1-none --package "$CONTRACT")
    WASM_PATH="$REPO_ROOT/target/wasm32v1-none/release/$WASM_FILE"
else
    say "step 1/7: skipping build"
    [ -n "$WASM_PATH" ] || WASM_PATH="$REPO_ROOT/target/wasm32v1-none/release/$WASM_FILE"
fi
[ -f "$WASM_PATH" ] || die "wasm not found: $WASM_PATH"

# ------------------------------------------------------- 2. size budget ----
WASM_SIZE="$(wc -c < "$WASM_PATH" | tr -d ' ')"
say "step 2/7: wasm size $WASM_SIZE bytes (budget $MAX_WASM_BYTES)"
[ "$WASM_SIZE" -le "$MAX_WASM_BYTES" ] \
    || die "wasm exceeds size budget ($WASM_SIZE > $MAX_WASM_BYTES); see runbook §2 item 4"

# ------------------------------------------------------------- 3. hash -----
EXPECTED_HASH="$(SHA256 "$WASM_PATH")"
say "step 3/7: local wasm hash (sha256): $EXPECTED_HASH"
say "          >>> confirm this equals the hash reviewers reproduced from the"
say "              audited tag BEFORE continuing (runbook §3) <<<"

# ------------------------------------------------- pre-upgrade snapshot ----
# NOTE: stellar-cli writes informational logs (incl. explorer links carrying
# 64-hex transaction hashes) to stderr; parse results from stdout ONLY.
PRE_VERSION=""
if [ "$DRY_RUN" -eq 0 ]; then
    if pre_out="$(stellar contract invoke --id "$CONTRACT_ID" \
            --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
            -- get_schema_version 2>/dev/null)"; then
        PRE_VERSION="$(echo "$pre_out" | grep -Eo '[0-9]+' | tail -1 || true)"
    fi
    say "pre-upgrade schema version: ${PRE_VERSION:-unknown (legacy pre-versioning code)}"
fi

# ------------------------------------------------------------- dry run -----
if [ "$DRY_RUN" -eq 1 ]; then
    cat <<EOF

DRY RUN — commands that would be executed:

  stellar contract upload \\
    --wasm "$WASM_PATH" --optimize=false \\
    --source-account "$SOURCE_ACCOUNT" --network "$NETWORK"
  # -> expect hash $EXPECTED_HASH (script aborts on mismatch)

  stellar contract invoke --id "$CONTRACT_ID" \\
    --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send yes \\
    -- upgrade --new-wasm-hash "$EXPECTED_HASH"
EOF
    if [ "$RUN_MIGRATE" -eq 1 ]; then
        cat <<EOF

  stellar contract invoke --id "$CONTRACT_ID" \\
    --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send yes \\
    -- migrate
EOF
    fi
    cat <<EOF

  stellar contract invoke --id "$CONTRACT_ID" \\
    --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \\
    -- get_schema_version      # expect ${EXPECTED_SCHEMA_VERSION:-<unchanged>}
  stellar contract fetch --id "$CONTRACT_ID" --network "$NETWORK" --out-file /tmp/post_upgrade.wasm
  sha256sum /tmp/post_upgrade.wasm   # expect $EXPECTED_HASH

Nothing was submitted.
EOF
    exit 0
fi

# ------------------------------------------------------------- 4. upload ---
say "step 4/7: uploading wasm (--optimize=false so bytes stay identical to the reviewed artifact)"
UPLOAD_LOG="$(mktemp -t lafiya-upload-log-XXXXXX)"
if ! upload_out="$(stellar contract upload --wasm "$WASM_PATH" --optimize=false \
        --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" 2>"$UPLOAD_LOG")"; then
    cat "$UPLOAD_LOG" >&2; rm -f "$UPLOAD_LOG"
    die "wasm upload failed"
fi
rm -f "$UPLOAD_LOG"
UPLOADED_HASH="$(printf '%s' "$upload_out" | grep -Eom1 '[0-9a-f]{64}' || true)"
[ -n "$UPLOADED_HASH" ] || die "could not parse wasm hash from upload output: $upload_out"
say "          network-reported hash:    $UPLOADED_HASH"
[ "$UPLOADED_HASH" = "$EXPECTED_HASH" ] \
    || die "hash mismatch (local $EXPECTED_HASH != uploaded $UPLOADED_HASH) — aborting; see runbook §3"

# ------------------------------------------------------------ 5. upgrade ---
say "step 5/7: submitting upgrade(new_wasm_hash=$UPLOADED_HASH)"
stellar contract invoke --id "$CONTRACT_ID" \
    --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send yes \
    -- upgrade --new-wasm-hash "$UPLOADED_HASH"
say "          upgrade transaction accepted"

# ------------------------------------------------------ 6. migrate (opt) ---
if [ "$RUN_MIGRATE" -eq 1 ]; then
    say "step 6/7: submitting migrate() for pending schema migration"
    stellar contract invoke --id "$CONTRACT_ID" \
        --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send yes \
        -- migrate
else
    say "step 6/7: skipped (code-only upgrade; pass --run-migrate for schema-changing ones)"
fi

# ------------------------------------------------------------- 7. verify ---
say "step 7/7: post-upgrade verification"
VERSION_LOG="$(mktemp -t lafiya-version-log-XXXXXX)"
if ! version_out="$(stellar contract invoke --id "$CONTRACT_ID" \
        --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
        -- get_schema_version 2>"$VERSION_LOG")"; then
    cat "$VERSION_LOG" >&2; rm -f "$VERSION_LOG"
    die "could not read get_schema_version post-upgrade"
fi
rm -f "$VERSION_LOG"
POST_VERSION="$(echo "$version_out" | grep -Eo '[0-9]+' | tail -1 || true)"
[ -n "$POST_VERSION" ] || die "could not parse schema version post-upgrade: $version_out"
say "          get_schema_version: ${PRE_VERSION:-LEGACY/unknown} -> $POST_VERSION"

if [ -n "$EXPECTED_SCHEMA_VERSION" ]; then
    [ "$POST_VERSION" = "$EXPECTED_SCHEMA_VERSION" ] \
        || die "schema version mismatch after upgrade: got $POST_VERSION, expected $EXPECTED_SCHEMA_VERSION"
fi

TMP_WASM="$(mktemp -t lafiya-post-upgrade-XXXXXX.wasm)"
trap 'rm -f "$TMP_WASM"' EXIT
stellar contract fetch --id "$CONTRACT_ID" --network "$NETWORK" --out-file "$TMP_WASM" >/dev/null 2>&1 \
    || die "could not fetch on-chain wasm for verification"
ONCHAIN_HASH="$(SHA256 "$TMP_WASM")"
say "          on-chain wasm hash:  $ONCHAIN_HASH"
[ "$ONCHAIN_HASH" = "$UPLOADED_HASH" ] \
    || die "on-chain code does not match uploaded hash ($ONCHAIN_HASH != $UPLOADED_HASH)"

say "SUCCESS: $CONTRACT ($CONTRACT_ID) now runs wasm $UPLOADED_HASH at schema version $POST_VERSION"
say "next: run the state spot-checks in runbook §5.2 (known-attester / known-attestation reads)"
