#!/usr/bin/env bash
# One-command audit environment: build, test, fuzz, budget and coverage
# reports, and (with --deploy) a deploy to a local Stellar network.
#
#   scripts/audit-env.sh [--deploy]
#
# Reports are written to target/audit/. See audit/environment.md.
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=target/audit
mkdir -p "$OUT"
DEPLOY=false
[[ "${1:-}" == "--deploy" ]] && DEPLOY=true

step() { printf '\n==> %s\n' "$*"; }

step "Toolchain"
rustc --version | tee "$OUT/toolchain.txt"
cargo --version | tee -a "$OUT/toolchain.txt"
git rev-parse HEAD | tee "$OUT/commit.txt"

step "Build contracts (wasm32v1-none)"
make wasm-contracts
sha256sum target/wasm32v1-none/release/*.wasm | tee "$OUT/wasm-sha256.txt"

step "Format and lint"
make fmt-check clippy

step "Unit and integration tests"
cargo test --workspace --locked 2>&1 | tee "$OUT/test.txt"

step "Property tests (PROPTEST_CASES=${PROPTEST_CASES:=1024})"
PROPTEST_CASES="$PROPTEST_CASES" cargo test --workspace --locked fuzz_test 2>&1 | tee "$OUT/fuzz.txt"

step "Budget report"
make bench 2>&1 | tee "$OUT/budget.txt"

step "Coverage report"
if cargo llvm-cov --version >/dev/null 2>&1; then
  cargo llvm-cov --workspace --locked --summary-only 2>&1 | tee "$OUT/coverage.txt"
else
  echo "cargo-llvm-cov not installed; skipping (cargo install --locked cargo-llvm-cov)" | tee "$OUT/coverage.txt"
fi

if $DEPLOY; then
  step "Deploy to a local network"
  stellar container start local
  ./tests/integration/run.sh 2>&1 | tee "$OUT/local-deploy.txt"
fi

step "Done. Reports in $OUT/"
ls -1 "$OUT"
