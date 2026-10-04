#!/usr/bin/env bash
# Lafiya - Shared Network Config Loader
# Shared by deploy script and admin CLI.
# Resolves the config for a given --network name via lafiya-cli.
#
# Provides:
#   load_network_config <network> [--config <path>]
#   -> sets:
#     LAFIYA_NETWORK
#     LAFIYA_RPC_URL
#     LAFIYA_NETWORK_PASSPHRASE
#     LAFIYA_ATTESTER_REGISTRY_ID
#     LAFIYA_ATTESTATION_REGISTRY_ID
#
# Usage:
#   source ./scripts/lib/config.sh
#   load_network_config "testnet"
#
# No secrets are ever loaded from the config file.

set -euo pipefail

# Resolve repo root (one level up from scripts/lib)
_LAFIYA_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_LAFIYA_REPO_ROOT="$(cd "$_LAFIYA_LIB_DIR/../.." && pwd)"
_LAFIYA_DEFAULT_CONFIG="$_LAFIYA_REPO_ROOT/config/networks.toml"

# All parsing, validation and layered overrides (--set is CLI-only, then
# LAFIYA_<NETWORK>_<KEY> env vars, config/networks.local.toml, networks.toml)
# are done by lafiya-cli, so the Bash and Rust paths share one set of rules.
# Uses $LAFIYA_CLI, else `lafiya-cli` on PATH, else `cargo run -p lafiya-cli`.
_lafiya_cli() {
    if [[ -n "${LAFIYA_CLI:-}" ]]; then
        "$LAFIYA_CLI" "$@"
    elif command -v lafiya-cli >/dev/null 2>&1; then
        lafiya-cli "$@"
    elif command -v cargo >/dev/null 2>&1; then
        cargo run --quiet --manifest-path "$_LAFIYA_REPO_ROOT/Cargo.toml" -p lafiya-cli -- "$@"
    else
        echo "ERROR: lafiya-cli not found. Install Rust (https://rustup.rs) or set LAFIYA_CLI." >&2
        return 1
    fi
}

load_network_config() {
    local network="${1:-}"
    local config_path="${2:-$_LAFIYA_DEFAULT_CONFIG}"

    if [[ -z "$network" ]]; then
        echo "Usage: load_network_config <network> [config_path]" >&2
        return 1
    fi

    if [[ ! -f "$config_path" ]]; then
        echo "ERROR: Config file not found: $config_path" >&2
        return 1
    fi

    local resolved
    resolved="$(_lafiya_cli --network "$network" --config "$config_path" config env)" || return 1
    # Values are single-quoted by lafiya-cli, so eval is safe.
    eval "$resolved"

    if [[ -z "$LAFIYA_RPC_URL" ]]; then
        echo "ERROR: rpc_url empty for network '$network'" >&2
        return 1
    fi
    if [[ -z "$LAFIYA_NETWORK_PASSPHRASE" ]]; then
        echo "ERROR: network_passphrase empty for network '$network'" >&2
        return 1
    fi

    export LAFIYA_NETWORK LAFIYA_CONFIG_PATH LAFIYA_RPC_URL LAFIYA_NETWORK_PASSPHRASE
    export LAFIYA_ATTESTER_REGISTRY_ID LAFIYA_ATTESTATION_REGISTRY_ID
}

list_networks() {
    local config_path="${1:-$_LAFIYA_DEFAULT_CONFIG}"
    _lafiya_cli --config "$config_path" config list | sed -n 's/^  - //p'
}

print_network_config() {
    local network="${1:-}"
    local config_path="${2:-$_LAFIYA_DEFAULT_CONFIG}"
    if [[ -z "$network" ]]; then
        echo "Usage: print_network_config <network>" >&2
        return 1
    fi
    load_network_config "$network" "$config_path"
    echo "Network: $LAFIYA_NETWORK"
    echo "Config: $LAFIYA_CONFIG_PATH"
    echo "RPC URL: $LAFIYA_RPC_URL"
    echo "Passphrase: $LAFIYA_NETWORK_PASSPHRASE"
    echo "Attester Registry: ${LAFIYA_ATTESTER_REGISTRY_ID:-<not deployed>}"
    echo "Attestation Registry: ${LAFIYA_ATTESTATION_REGISTRY_ID:-<not deployed>}"
}

# ---------------------------------------------------------------------------
# Shell-safe single-quote escaping (issue #396)
# ---------------------------------------------------------------------------
# shell_quote VALUE  -- wraps VALUE in single quotes, escaping embedded ' as '\''
# This prevents command injection when values are eval'd or sourced.
shell_quote() {
    local value="$1"
    # Replace each ' with '\'', then wrap the whole thing in single quotes.
    local escaped="${value//\'/\'\\\'\'}"
    printf "'%s'" "$escaped"
}

# print_env_exports  -- print shell export lines for all LAFIYA_ vars, safely quoted
# Usage: load_network_config testnet; print_env_exports
print_env_exports() {
    printf 'export LAFIYA_NETWORK=%s\n'              "$(shell_quote "${LAFIYA_NETWORK:-}")"
    printf 'export LAFIYA_RPC_URL=%s\n'              "$(shell_quote "${LAFIYA_RPC_URL:-}")"
    printf 'export LAFIYA_NETWORK_PASSPHRASE=%s\n'   "$(shell_quote "${LAFIYA_NETWORK_PASSPHRASE:-}")"
    printf 'export LAFIYA_ATTESTER_REGISTRY_ID=%s\n' "$(shell_quote "${LAFIYA_ATTESTER_REGISTRY_ID:-}")"
    printf 'export LAFIYA_ATTESTATION_REGISTRY_ID=%s\n' "$(shell_quote "${LAFIYA_ATTESTATION_REGISTRY_ID:-}")"
}

# If sourced directly for testing: allow CLI
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    if [[ "${1:-}" == "--list" ]]; then
        list_networks "${2:-}"
    elif [[ "${1:-}" == "--env" ]]; then
        # Print safely-quoted export lines: eval $(config.sh --env testnet)
        load_network_config "${2:-}" "${3:-}"
        print_env_exports
    elif [[ -n "${1:-}" ]]; then
        print_network_config "$1" "${2:-}"
    else
        echo "Usage: $0 <network> [config_path] | $0 --list [config_path] | $0 --env <network> [config_path]" >&2
        exit 1
    fi
fi
