#!/usr/bin/env bash
# Lafiya - Shared Input Validation
# Sourced by scripts/admin.sh and scripts/deploy.sh so operator input is checked
# locally, before anything is handed to the stellar CLI.
#
# Provides:
#   lafiya_validate_network_name <name>
#   lafiya_validate_address <label> <value>          # G... or C...
#   lafiya_validate_account_address <label> <value>  # G... only
#   lafiya_validate_contract_id <label> <value>      # C... only
#   lafiya_validate_record_hash <value>              # 64 hex chars
#   lafiya_validate_source_account <value>           # identity name or G...
#   lafiya_require_deployment <network> <config> <attester_id> <attestation_id> <needed>
#
# Every function prints an actionable error to stderr and returns 1 on failure.
# Checks are structural (prefix, length, charset); the Rust CLI additionally
# verifies the strkey CRC16 checksum. No secrets are ever echoed.

# Strkey alphabet is RFC 4648 base32 (uppercase letters and digits 2-7), 56 chars.
_LAFIYA_STRKEY_RE='^[A-Z2-7]{56}$'

lafiya_validation_error() {
    echo "ERROR: $1" >&2
    return 1
}

lafiya_validate_network_name() {
    local name="${1:-}"
    if [[ -z "$name" ]]; then
        lafiya_validation_error "network name must not be empty (use --network <name>)"
        return 1
    fi
    if (( ${#name} > 32 )); then
        lafiya_validation_error "network name must be at most 32 characters, got ${#name}"
        return 1
    fi
    if [[ ! "$name" =~ ^[A-Za-z0-9_-]+$ ]]; then
        lafiya_validation_error "network name '$name' may only contain letters, digits, '-' and '_'"
        return 1
    fi
}

# Internal: shared strkey shape check. $3 is a regex of allowed first characters.
_lafiya_validate_strkey() {
    local label="$1"
    local value="${2:-}"
    local prefix_re="$3"
    local prefix_help="$4"

    if [[ -z "$value" ]]; then
        lafiya_validation_error "$label must not be empty"
        return 1
    fi
    if (( ${#value} != 56 )); then
        lafiya_validation_error "$label must be a 56-character Stellar $prefix_help, got ${#value} characters"
        return 1
    fi
    if [[ ! "$value" =~ $_LAFIYA_STRKEY_RE ]]; then
        lafiya_validation_error "$label must contain only A-Z and 2-7 (Stellar strkey alphabet)"
        return 1
    fi
    if [[ ! "$value" =~ $prefix_re ]]; then
        lafiya_validation_error "$label must be a Stellar $prefix_help"
        return 1
    fi
}

lafiya_validate_address() {
    _lafiya_validate_strkey "${1:-address}" "${2:-}" '^[GC]' "address (G... account or C... contract)"
}

lafiya_validate_account_address() {
    _lafiya_validate_strkey "${1:-address}" "${2:-}" '^G' "account address (G...)"
}

lafiya_validate_contract_id() {
    _lafiya_validate_strkey "${1:-contract id}" "${2:-}" '^C' "contract id (C...)"
}

lafiya_validate_record_hash() {
    local value="${1:-}"
    if [[ -z "$value" ]]; then
        lafiya_validation_error "record hash must not be empty"
        return 1
    fi
    if (( ${#value} != 64 )); then
        lafiya_validation_error "record hash must be 64 hex characters (32-byte hash), got ${#value}"
        return 1
    fi
    if [[ ! "$value" =~ ^[0-9a-fA-F]{64}$ ]]; then
        lafiya_validation_error "record hash must contain only hex characters (0-9, a-f)"
        return 1
    fi
}

# A --source is either a stellar CLI identity name or a G... address.
# Secret keys (S...) are rejected: secrets belong in identities or the environment.
lafiya_validate_source_account() {
    local value="${1:-}"
    if [[ -z "$value" ]]; then
        lafiya_validation_error "source account must not be empty"
        return 1
    fi
    if [[ "$value" =~ ^G && ${#value} -eq 56 ]]; then
        lafiya_validate_account_address "source" "$value"
        return
    fi
    if [[ "$value" =~ ^S[A-Z2-7]{55}$ ]]; then
        lafiya_validation_error "source must not be a secret key - use a stellar identity name or a G... address"
        return 1
    fi
    if (( ${#value} > 64 )); then
        lafiya_validation_error "source identity name must be at most 64 characters, got ${#value}"
        return 1
    fi
    if [[ ! "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
        lafiya_validation_error "source identity name may only contain letters, digits, '.', '-' and '_'"
        return 1
    fi
}

# Report deployment completeness for a network profile.
# $5 selects what the caller needs: attester | attestation | any
lafiya_require_deployment() {
    local network="$1"
    local config_path="$2"
    local attester_id="${3:-}"
    local attestation_id="${4:-}"
    local needed="${5:-any}"

    local missing=()
    if [[ -z "$attester_id" ]]; then
        missing+=("attester_registry")
    fi
    if [[ -z "$attestation_id" ]]; then
        missing+=("attestation_registry")
    fi

    if (( ${#missing[@]} == 1 )); then
        echo "WARNING: network '$network' is partially deployed - missing ${missing[0]} in $config_path" >&2
    fi

    local wanted="" label=""
    case "$needed" in
        attester) wanted="$attester_id"; label="attester_registry" ;;
        attestation) wanted="$attestation_id"; label="attestation_registry" ;;
        any) wanted="${attester_id}${attestation_id}"; label="attester_registry/attestation_registry" ;;
        *)
            lafiya_validation_error "unknown deployment requirement '$needed'"
            return 1
            ;;
    esac

    if [[ -z "$wanted" ]]; then
        lafiya_validation_error "$label contract ID is not set for network '$network' in $config_path
       Deploy first: ./scripts/deploy.sh --network $network
       Then record the contract id under [$network.contracts] in $config_path"
        return 1
    fi

    # Recorded IDs must still be well formed before they reach the stellar CLI.
    if [[ -n "$attester_id" ]]; then
        lafiya_validate_contract_id "attester_registry ($network)" "$attester_id" || return 1
    fi
    if [[ -n "$attestation_id" ]]; then
        lafiya_validate_contract_id "attestation_registry ($network)" "$attestation_id" || return 1
    fi
}

# Self-test: ./scripts/lib/validate.sh --self-test
# Runs entirely offline; no stellar CLI or network access required.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    if [[ "${1:-}" != "--self-test" ]]; then
        echo "Usage: $0 --self-test" >&2
        exit 1
    fi

    _valid_account="GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ"
    _valid_contract="CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA"
    _failures=0

    _expect_ok() {
        if "$@" 2>/dev/null; then
            echo "ok    : $*"
        else
            echo "FAIL  : expected success: $*" >&2
            _failures=$((_failures + 1))
        fi
    }

    _expect_err() {
        if "$@" 2>/dev/null; then
            echo "FAIL  : expected failure: $*" >&2
            _failures=$((_failures + 1))
        else
            echo "ok    : rejected: $*"
        fi
    }

    _expect_ok lafiya_validate_network_name testnet
    _expect_err lafiya_validate_network_name ""
    _expect_err lafiya_validate_network_name "test net"
    _expect_err lafiya_validate_network_name "../etc"

    _expect_ok lafiya_validate_address attester "$_valid_account"
    _expect_ok lafiya_validate_address attester "$_valid_contract"
    _expect_err lafiya_validate_address attester ""
    _expect_err lafiya_validate_address attester "GABC"
    _expect_err lafiya_validate_address attester "$(echo "$_valid_account" | tr 'A-Z' 'a-z')"
    _expect_err lafiya_validate_address attester "M${_valid_account:1}"

    _expect_ok lafiya_validate_contract_id attester_registry "$_valid_contract"
    _expect_err lafiya_validate_contract_id attester_registry "$_valid_account"

    _expect_ok lafiya_validate_record_hash "$(printf 'a%.0s' {1..64})"
    _expect_err lafiya_validate_record_hash ""
    _expect_err lafiya_validate_record_hash "abc"
    _expect_err lafiya_validate_record_hash "0x$(printf 'a%.0s' {1..62})"

    _expect_ok lafiya_validate_source_account deployer
    _expect_ok lafiya_validate_source_account "$_valid_account"
    _expect_err lafiya_validate_source_account ""
    _expect_err lafiya_validate_source_account "admin; rm -rf /"
    _expect_err lafiya_validate_source_account "S${_valid_account:1}"

    _expect_ok lafiya_require_deployment testnet cfg.toml "$_valid_contract" "$_valid_contract" any
    _expect_ok lafiya_require_deployment testnet cfg.toml "$_valid_contract" "" attester
    _expect_err lafiya_require_deployment testnet cfg.toml "$_valid_contract" "" attestation
    _expect_err lafiya_require_deployment testnet cfg.toml "" "" any
    _expect_err lafiya_require_deployment testnet cfg.toml "CA6P..." "" attester

    if (( _failures > 0 )); then
        echo "$_failures validation self-test(s) failed" >&2
        exit 1
    fi
    echo "All validation self-tests passed."
fi
