#!/usr/bin/env bash
# One-command contributor setup: `make first-run`.
# Detects prerequisites (with exact remediation per OS), then smoke-builds and
# tests one contract and one TypeScript package. Set LAFIYA_FIRST_RUN_TIMING=1
# to print machine-readable, anonymous timing (no identifiers) for CI.
set -uo pipefail
cd "$(dirname "$0")/.."

start=$(date +%s)
missing=0
case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) os=other ;;
esac

need() { # need <cmd> <remediation>
  if command -v "$1" >/dev/null 2>&1; then
    echo "ok       $1"
  else
    echo "MISSING  $1 -> $2"
    missing=1
  fi
}

pkg() { # pkg <apt> <brew>
  if [ "$os" = macos ]; then echo "brew install $2"; else echo "sudo apt-get install -y $1"; fi
}

echo "== Prerequisites ($os) =="
need rustup "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
need cargo "install rustup (above), then restart your shell"
need stellar "cargo install --locked stellar-cli --features opt"
need node "$(pkg nodejs node) (Node 22 LTS)"
need pnpm "corepack enable && corepack prepare pnpm@latest --activate"
need python3 "$(pkg python3 python@3.12)"
command -v psql >/dev/null 2>&1 || echo "optional psql missing (indexer only) -> $(pkg postgresql-client libpq)"
if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -q wasm32v1-none; then
  echo "MISSING  wasm32v1-none target -> rustup target add wasm32v1-none"
  missing=1
fi
if [ "$missing" -ne 0 ]; then
  echo "Install the items marked MISSING, then re-run: make first-run" >&2
  echo "(The devcontainer in .devcontainer/ provides all of these.)" >&2
  exit 1
fi

step() { # step <description> <remediation> <cmd...>
  local what=$1 fix=$2
  shift 2
  echo "== $what =="
  if ! "$@"; then
    echo "FAILED: $what -> $fix" >&2
    exit 1
  fi
}

step "Contract smoke test (attester-registry)" \
  "run 'cargo test -p attester-registry' for details; ensure rust-toolchain.toml was honoured (rustup show)" \
  cargo test -p attester-registry --lib
step "Wasm build (attester-registry)" \
  "rustup target add wasm32v1-none" \
  cargo build -p attester-registry --release --target wasm32v1-none
step "TypeScript smoke test (verifier-sdk)" \
  "run 'pnpm install' in packages/verifier-sdk, then 'pnpm test'" \
  bash -c 'cd packages/verifier-sdk && pnpm install --frozen-lockfile && pnpm test'

elapsed=$(( $(date +%s) - start ))
echo "first-run OK in ${elapsed}s"
if [ "${LAFIYA_FIRST_RUN_TIMING:-}" = 1 ]; then
  echo "first_run_seconds=${elapsed}"
fi
