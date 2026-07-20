#!/usr/bin/env bash
# Lafiya - Shared Network Config Loader
# Shared by deploy script and admin CLI.
# Parses config/networks.toml for a given --network name.
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

# Internal: use python3 to parse TOML robustly
_lafiya_parse_toml() {
    local network="$1"
    local config_path="$2"
    local field="$3"

    python3 - "$network" "$config_path" "$field" <<'PY'
import sys
network = sys.argv[1]
config_path = sys.argv[2]
field = sys.argv[3]

# Python 3.11+ has tomllib built-in
try:
    import tomllib
except ModuleNotFoundError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Need python3.11+ with tomllib or pip install tomli", file=sys.stderr)
        sys.exit(2)

with open(config_path, "rb") as f:
    data = tomllib.load(f)

if network not in data:
    print(f"ERROR: network '{network}' not found in {config_path}", file=sys.stderr)
    print(f"Available networks: {', '.join(sorted(data.keys()))}", file=sys.stderr)
    sys.exit(1)

net_cfg = data[network]

mapping = {
    "rpc_url": net_cfg.get("rpc_url", ""),
    "network_passphrase": net_cfg.get("network_passphrase", ""),
    "attester_registry": net_cfg.get("contracts", {}).get("attester_registry", ""),
    "attestation_registry": net_cfg.get("contracts", {}).get("attestation_registry", ""),
}

if field not in mapping:
    print(f"ERROR: unknown field '{field}'", file=sys.stderr)
    sys.exit(1)

print(mapping[field])
PY
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

    # Exported globals
    LAFIYA_NETWORK="$network"
    LAFIYA_CONFIG_PATH="$config_path"
    LAFIYA_RPC_URL="$(_lafiya_parse_toml "$network" "$config_path" "rpc_url")"
    LAFIYA_NETWORK_PASSPHRASE="$(_lafiya_parse_toml "$network" "$config_path" "network_passphrase")"
    LAFIYA_ATTESTER_REGISTRY_ID="$(_lafiya_parse_toml "$network" "$config_path" "attester_registry")"
    LAFIYA_ATTESTATION_REGISTRY_ID="$(_lafiya_parse_toml "$network" "$config_path" "attestation_registry")"

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
    python3 - "$config_path" <<'PY'
import sys
config_path = sys.argv[1]
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
with open(config_path, "rb") as f:
    data = tomllib.load(f)
for name in sorted(data.keys()):
    print(name)
PY
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

# If sourced directly for testing: allow CLI
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    if [[ "${1:-}" == "--list" ]]; then
        list_networks "${2:-}"
    elif [[ -n "${1:-}" ]]; then
        print_network_config "$1" "${2:-}"
    else
        echo "Usage: $0 <network> [config_path] | $0 --list [config_path]" >&2
        exit 1
    fi
fi
