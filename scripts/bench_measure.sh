#!/usr/bin/env bash
# Build the contract wasm of the checkout in the current directory and print
# its lafiya-bench metrics (JSON) on stdout. Used by scripts/bench_history.py
# (backfill) and scripts/bench_bisect.py, which run it in a throwaway worktree
# of each commit with the harness built once from the current tree.
#
#   LAFIYA_BENCH      lafiya-bench binary (default: <repo>/target/debug/lafiya-bench)
#   BENCH_TARGET_DIR  cargo target dir shared across commits (default: <repo>/target/bench)
#
# The harness is a dev-profile build on purpose: the release profile aborts on
# panic, and the harness relies on unwinding to report a failing scenario.
# Soroban's cost model is deterministic, so the profile does not change the
# numbers.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
bench="${LAFIYA_BENCH:-$repo/target/debug/lafiya-bench}"
export CARGO_TARGET_DIR="${BENCH_TARGET_DIR:-$repo/target/bench}"
# Plain `cargo build` of a soroban-sdk 28 contract needs this (stellar-cli sets
# it). Every commit is built the same way, so the series stays comparable.
export SOROBAN_SDK_BUILD_SYSTEM_SUPPORTS_SPEC_SHAKING_V2=1

# Some commits on main carry an unparseable .cargo/config.toml (a merged local
# Windows linker section). It only holds aliases and host linker settings, so
# drop it rather than lose the commit.
if [ -f .cargo/config.toml ] &&
  ! python3 -c 'import sys, tomllib; tomllib.load(open(sys.argv[1], "rb"))' .cargo/config.toml 2>/dev/null; then
  rm .cargo/config.toml
fi

out="$CARGO_TARGET_DIR/wasm32v1-none/release"
# Drop wasm left by the previous commit so a contract that does not exist here
# is reported missing instead of measured stale.
rm -f "$out"/*.wasm

# One build per contract: a contract that does not compile at this commit is
# reported missing by the harness instead of hiding the others. No --locked:
# some older commits carry a Cargo.lock out of date with their manifests.
for dir in contracts/*/; do
  cargo build --quiet --release --target wasm32v1-none -p "$(basename "$dir")" >&2 ||
    echo "bench_measure: $(basename "$dir") does not build at this commit" >&2
done
"$bench" --wasm-dir "$out"
