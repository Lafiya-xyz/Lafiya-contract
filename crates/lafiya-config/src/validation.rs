//! Offline validation for operator supplied values.
//!
//! Every check in this module runs locally: no network access, no stellar CLI,
//! no secrets. The goal is to reject malformed input *before* it reaches the
//! stellar CLI so operators get an early, actionable error instead of a late,
//! cryptic one.
//!
//! Stellar strkeys (`G...` accounts, `C...` contracts) are validated all the way
//! down to their CRC16 checksum, so a single mistyped character is caught here.

use thiserror::Error;

/// Length in characters of an ed25519 / contract strkey.
const STRKEY_LEN: usize = 56;
/// Decoded strkey layout: 1 version byte + 32 payload bytes + 2 checksum bytes.
const STRKEY_DECODED_LEN: usize = 35;
/// Version byte for an ed25519 public key (`G...`).
const VERSION_BYTE_ACCOUNT: u8 = 6 << 3;
/// Version byte for a contract id (`C...`).
const VERSION_BYTE_CONTRACT: u8 = 2 << 3;
/// Hex characters in a 32-byte record hash.
const RECORD_HASH_HEX_LEN: usize = 64;
/// Upper bound for a network name; generous but keeps errors readable.
const MAX_NETWORK_NAME_LEN: usize = 32;
/// Upper bound for a stellar CLI identity name.
const MAX_SOURCE_LEN: usize = 64;

/// What kind of strkey a value is expected to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// Account address only (`G...`).
    Account,
    /// Contract id only (`C...`).
    Contract,
    /// Either an account or a contract, i.e. anything a Soroban `Address` accepts.
    AccountOrContract,
}

impl AddressKind {
    fn expected_prefix(self) -> &'static str {
        match self {
            AddressKind::Account => "G",
            AddressKind::Contract => "C",
            AddressKind::AccountOrContract => "G or C",
        }
    }

    fn accepts(self, version_byte: u8) -> bool {
        match self {
            AddressKind::Account => version_byte == VERSION_BYTE_ACCOUNT,
            AddressKind::Contract => version_byte == VERSION_BYTE_CONTRACT,
            AddressKind::AccountOrContract => {
                version_byte == VERSION_BYTE_ACCOUNT || version_byte == VERSION_BYTE_CONTRACT
            }
        }
    }
}

/// A validation failure. Messages name the offending field and what was expected,
/// never the surrounding secret material (identities, keys) that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },

    #[error("{field} must be {expected} characters, got {actual}")]
    Length {
        field: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("{field} must be at most {max} characters, got {actual}")]
    TooLong {
        field: &'static str,
        max: usize,
        actual: usize,
    },

    #[error("{field} contains invalid character '{character}' (allowed: {allowed})")]
    Charset {
        field: &'static str,
        character: char,
        allowed: &'static str,
    },

    #[error("{field} must start with {expected} (Stellar strkey prefix)")]
    Prefix {
        field: &'static str,
        expected: &'static str,
    },

    #[error("{field} has an invalid checksum - check for a typo in the address")]
    Checksum { field: &'static str },

    #[error("{field} must be a http:// or https:// URL, got '{value}'")]
    RpcUrl { field: &'static str, value: String },

    #[error(
        "network profile '{profile}' is mainnet but {field} looks like a {found} value; \
         refusing to sign a mixed configuration"
    )]
    MixedNetworkConfig {
        profile: &'static str,
        field: &'static str,
        found: &'static str,
    },

    #[error(
        "network passphrase does not match profile '{profile}': expected '{expected}', got '{actual}'"
    )]
    PassphraseMismatch {
        profile: &'static str,
        expected: &'static str,
        actual: String,
    },
}

/// Well-known network passphrases, used to cross-check a profile before signing.
pub const PASSPHRASE_MAINNET: &str = "Public Global Stellar Network ; September 2015";
/// Testnet passphrase.
pub const PASSPHRASE_TESTNET: &str = "Test SDF Network ; September 2015";
/// Futurenet passphrase.
pub const PASSPHRASE_FUTURENET: &str = "Test SDF Future Network ; October 2022";
/// Local/standalone passphrase used by `stellar network container`.
pub const PASSPHRASE_LOCAL: &str = "Standalone Network ; February 2017";

/// The risk class of a network profile.
///
/// Mainnet is the only class that requires deliberate confirmation; everything
/// else is treated as disposable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkClass {
    /// Local/standalone development network.
    Local,
    /// Shared test network (testnet, futurenet, ...).
    Testnet,
    /// The public Stellar network.
    Mainnet,
}

impl NetworkClass {
    /// Classify a profile name. Unknown names are treated as testnet-like so
    /// they never accidentally gain mainnet privileges.
    pub fn from_profile(profile: &str) -> Self {
        match profile {
            "mainnet" | "public" => NetworkClass::Mainnet,
            "local" | "standalone" => NetworkClass::Local,
            _ => NetworkClass::Testnet,
        }
    }

    /// Whether mutating operations on this class need explicit confirmation.
    pub fn requires_confirmation(self) -> bool {
        matches!(self, NetworkClass::Mainnet)
    }

    /// The passphrase this class is expected to use, when it is well known.
    pub fn expected_passphrase(self) -> Option<&'static str> {
        match self {
            NetworkClass::Mainnet => Some(PASSPHRASE_MAINNET),
            NetworkClass::Testnet => None,
            NetworkClass::Local => Some(PASSPHRASE_LOCAL),
        }
    }
}

/// Validate a Stellar strkey of the requested kind (charset, length, prefix, checksum).
pub fn validate_strkey(
    field: &'static str,
    value: &str,
    kind: AddressKind,
) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if value.len() != STRKEY_LEN {
        return Err(ValidationError::Length {
            field,
            expected: STRKEY_LEN,
            actual: value.chars().count(),
        });
    }
    if let Some(c) = value.chars().find(|c| !is_base32_char(*c)) {
        return Err(ValidationError::Charset {
            field,
            character: c,
            allowed: "A-Z and 2-7",
        });
    }

    let decoded = decode_base32(value).ok_or(ValidationError::Charset {
        field,
        character: '?',
        allowed: "A-Z and 2-7",
    })?;
    if decoded.len() != STRKEY_DECODED_LEN {
        return Err(ValidationError::Length {
            field,
            expected: STRKEY_LEN,
            actual: value.chars().count(),
        });
    }

    if !kind.accepts(decoded[0]) {
        return Err(ValidationError::Prefix {
            field,
            expected: kind.expected_prefix(),
        });
    }

    let (body, checksum) = decoded.split_at(STRKEY_DECODED_LEN - 2);
    let expected = u16::from_le_bytes([checksum[0], checksum[1]]);
    if crc16_xmodem(body) != expected {
        return Err(ValidationError::Checksum { field });
    }

    Ok(())
}

/// Validate an address that a contract call accepts (`G...` account or `C...` contract).
pub fn validate_address(field: &'static str, value: &str) -> Result<(), ValidationError> {
    validate_strkey(field, value, AddressKind::AccountOrContract)
}

/// Validate an account address (`G...`), e.g. a contract admin.
pub fn validate_account_address(field: &'static str, value: &str) -> Result<(), ValidationError> {
    validate_strkey(field, value, AddressKind::Account)
}

/// Validate a deployed contract id (`C...`).
pub fn validate_contract_id(field: &'static str, value: &str) -> Result<(), ValidationError> {
    validate_strkey(field, value, AddressKind::Contract)
}

/// Validate a hex encoded 32-byte record hash (64 hex characters).
pub fn validate_record_hash(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if value.len() != RECORD_HASH_HEX_LEN {
        return Err(ValidationError::Length {
            field,
            expected: RECORD_HASH_HEX_LEN,
            actual: value.chars().count(),
        });
    }
    if let Some(c) = value.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(ValidationError::Charset {
            field,
            character: c,
            allowed: "0-9, a-f and A-F",
        });
    }
    Ok(())
}

/// Validate a network name before it is used as a config key or shell argument.
pub fn validate_network_name(value: &str) -> Result<(), ValidationError> {
    const FIELD: &str = "network";
    if value.is_empty() {
        return Err(ValidationError::Empty { field: FIELD });
    }
    if value.len() > MAX_NETWORK_NAME_LEN {
        return Err(ValidationError::TooLong {
            field: FIELD,
            max: MAX_NETWORK_NAME_LEN,
            actual: value.chars().count(),
        });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(ValidationError::Charset {
            field: FIELD,
            character: c,
            allowed: "a-z, A-Z, 0-9, '-' and '_'",
        });
    }
    Ok(())
}

/// Validate a `--source` value: either a stellar CLI identity name or a `G...` address.
///
/// Secret keys (`S...`) are deliberately not accepted here - secrets belong in
/// stellar identities or the environment, never on the command line.
pub fn validate_source_account(value: &str) -> Result<(), ValidationError> {
    const FIELD: &str = "source";
    if value.is_empty() {
        return Err(ValidationError::Empty { field: FIELD });
    }
    if value.starts_with('G') && value.len() == STRKEY_LEN {
        return validate_account_address(FIELD, value);
    }
    if value.len() > MAX_SOURCE_LEN {
        return Err(ValidationError::TooLong {
            field: FIELD,
            max: MAX_SOURCE_LEN,
            actual: value.chars().count(),
        });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.'))
    {
        return Err(ValidationError::Charset {
            field: FIELD,
            character: c,
            allowed: "a-z, A-Z, 0-9, '-', '_' and '.' (identity name) or a G... address",
        });
    }
    Ok(())
}

/// Validate a network passphrase before it is printed into a shell-evaluated context.
///
/// Requirements (issue #396):
/// - Non-empty.
/// - Printable ASCII only (0x20–0x7E).  No control characters, no null bytes.
///   Control characters (including newline, tab, carriage-return, bell, etc.)
///   can silently terminate an `export` statement or inject additional shell
///   commands when the passphrase is embedded in `eval`'d output.
/// - No shell metacharacters that survive single-quoting are rejected here
///   because single-quoting already neutralises them — but the printable-ASCII
///   gate keeps the door shut against non-printable injection vectors.
pub fn validate_passphrase(value: &str) -> Result<(), ValidationError> {
    const FIELD: &str = "network_passphrase";
    if value.is_empty() {
        return Err(ValidationError::Empty { field: FIELD });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !c.is_ascii() || (*c as u8) < 0x20 || *c == 0x7f as char)
    {
        return Err(ValidationError::Charset {
            field: FIELD,
            character: c,
            allowed: "printable ASCII (0x20–0x7E)",
        });
    }
    Ok(())
}

/// Validate an RPC URL: non-empty, http/https, and with a host component.
pub fn validate_rpc_url(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.starts_with('/') => Ok(()),
        _ => Err(ValidationError::RpcUrl {
            field,
            value: value.to_string(),
        }),
    }
}

/// Validate a hex encoded 32-byte wasm hash (64 hex characters).
///
/// This is the same shape as [`validate_record_hash`] but is kept as a distinct
/// entry point so drift verification can name the offending field precisely.
pub fn validate_wasm_hash(field: &'static str, value: &str) -> Result<(), ValidationError> {
    validate_record_hash(field, value)
}

/// Validate a contract wiring entry: a contract id (`C...`) that must not be empty.
pub fn validate_wiring_contract(field: &'static str, value: &str) -> Result<(), ValidationError> {
    validate_contract_id(field, value)
}

/// Verify that the configured passphrase matches the profile's expected passphrase.
///
/// This must run before signing so a mainnet profile can never be signed with a
/// testnet passphrase (or vice versa). Profiles without a well-known passphrase
/// (e.g. testnet, whose passphrase is stable but not pinned here) are accepted
/// as long as the passphrase itself is well formed.
pub fn validate_profile_passphrase(
    profile: &str,
    passphrase: &str,
) -> Result<(), ValidationError> {
    validate_passphrase(passphrase)?;
    let class = NetworkClass::from_profile(profile);
    if let Some(expected) = class.expected_passphrase() {
        if passphrase != expected {
            return Err(ValidationError::PassphraseMismatch {
                profile: profile_name(profile),
                expected,
                actual: passphrase.to_string(),
            });
        }
    }
    Ok(())
}

/// Detect a mixed configuration: a mainnet profile paired with contract ids or
/// other values that belong to a test network.
///
/// `deployment_network` is the network recorded in the deployment ledger for
/// the contract ids being used. When the profile is mainnet and the ledger says
/// the ids were deployed elsewhere, signing is refused.
pub fn validate_profile_deployment(
    profile: &str,
    deployment_network: &str,
) -> Result<(), ValidationError> {
    let class = NetworkClass::from_profile(profile);
    if !class.requires_confirmation() {
        return Ok(());
    }
    let deployed_class = NetworkClass::from_profile(deployment_network);
    if deployed_class != NetworkClass::Mainnet {
        return Err(ValidationError::MixedNetworkConfig {
            profile: profile_name(profile),
            field: "contract ids",
            found: deployment_network_name(deployment_network),
        });
    }
    Ok(())
}

/// Normalise a profile name into a `'static` label for error messages.
fn profile_name(profile: &str) -> &'static str {
    match NetworkClass::from_profile(profile) {
        NetworkClass::Mainnet => "mainnet",
        NetworkClass::Local => "local",
        NetworkClass::Testnet => "testnet",
    }
}

/// Normalise a deployment network name into a `'static` label for error messages.
fn deployment_network_name(network: &str) -> &'static str {
    match NetworkClass::from_profile(network) {
        NetworkClass::Mainnet => "mainnet",
        NetworkClass::Local => "local",
        NetworkClass::Testnet => "testnet",
    }
}

/// The set of config fields that drift verification must compare against chain.
///
/// Every field listed here is validated by this module before it is ever used,
/// so `config verify --against-chain` covers exactly the same surface that
/// offline validation does. Keep this list in sync with the validators above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftField {
    /// A deployed contract id (`C...`), e.g. `attester_registry`.
    ContractId,
    /// An admin account address (`G...`).
    Admin,
    /// A wiring entry pointing at another contract id (`C...`).
    Wiring,
    /// A hex encoded 32-byte wasm hash.
    WasmHash,
}

impl DriftField {
    /// Human readable label used in diffs and error messages.
    pub fn label(self) -> &'static str {
        match self {
            DriftField::ContractId => "contract id",
            DriftField::Admin => "admin",
            DriftField::Wiring => "wiring",
            DriftField::WasmHash => "wasm hash",
        }
    }

    /// Validate a value for this field, returning the same errors as the
    /// dedicated validators so drift checks and offline checks agree.
    pub fn validate(self, field: &'static str, value: &str) -> Result<(), ValidationError> {
        match self {
            DriftField::ContractId => validate_contract_id(field, value),
            DriftField::Admin => validate_account_address(field, value),
            DriftField::Wiring => validate_wiring_contract(field, value),
            DriftField::WasmHash => validate_wasm_hash(field, value),
        }
    }
}

/// Every field kind that drift verification covers, in a stable order.
pub const DRIFT_FIELDS: &[DriftField] = &[
    DriftField::ContractId,
    DriftField::Admin,
    DriftField::Wiring,
    DriftField::WasmHash,
];

fn is_base32_char(c: char) -> bool {
    c.is_ascii_uppercase() || ('2'..='7').contains(&c)
}

/// Decode an unpadded RFC 4648 base32 string. Returns `None` on a non-base32 char.
fn decode_base32(value: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len() * 5 / 8);
    let mut buffer: u16 = 0;
    let mut bits: u8 = 0;

    for c in value.chars() {
        let digit = match c {
            'A'..='Z' => c as u16 - 'A' as u16,
            '2'..='7' => c as u16 - '2' as u16 + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | digit;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Some(out)
}

/// CRC16-XModem, the checksum used by Stellar strkeys.
fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for byte in data {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published SEP-23 test vector for an ed25519 public key strkey.
    const VALID_ACCOUNT: &str = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
    /// Contract ids taken from docs/runbooks/contract-upgrade.md.
    const VALID_CONTRACT: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const VALID_CONTRACT_2: &str = "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX";

    #[test]
    fn accepts_valid_account_and_contract_strkeys() {
        assert!(validate_account_address("admin", VALID_ACCOUNT).is_ok());
        assert!(validate_contract_id("attester_registry", VALID_CONTRACT).is_ok());
        assert!(validate_contract_id("attestation_registry", VALID_CONTRACT_2).is_ok());
        assert!(validate_address("attester", VALID_ACCOUNT).is_ok());
        assert!(validate_address("attester", VALID_CONTRACT).is_ok());
    }

    #[test]
    fn rejects_empty_address() {
        assert_eq!(
            validate_address("attester", "").unwrap_err(),
            ValidationError::Empty { field: "attester" }
        );
    }

    #[test]
    fn rejects_wrong_length_address() {
        let truncated = &VALID_ACCOUNT[..40];
        assert!(matches!(
            validate_address("attester", truncated),
            Err(ValidationError::Length { actual: 40, .. })
        ));

        let padded = format!("{VALID_ACCOUNT}A");
        assert!(matches!(
            validate_address("attester", &padded),
            Err(ValidationError::Length { actual: 57, .. })
        ));
    }

    #[test]
    fn rejects_non_base32_characters() {
        // '0', '1', '8', '9' and lowercase are outside the strkey alphabet.
        let mut bad = VALID_ACCOUNT.to_string();
        bad.replace_range(10..11, "0");
        assert!(matches!(
            validate_address("attester", &bad),
            Err(ValidationError::Charset { character: '0', .. })
        ));

        assert!(matches!(
            validate_address("attester", &VALID_ACCOUNT.to_lowercase()),
            Err(ValidationError::Charset { .. })
        ));
    }

    #[test]
    fn rejects_wrong_prefix_for_kind() {
        assert!(matches!(
            validate_contract_id("attester_registry", VALID_ACCOUNT),
            Err(ValidationError::Prefix { expected: "C", .. })
        ));
        assert!(matches!(
            validate_account_address("admin", VALID_CONTRACT),
            Err(ValidationError::Prefix { expected: "G", .. })
        ));
    }

    #[test]
    fn rejects_muxed_and_seed_strkeys_as_addresses() {
        // 'M' (muxed) and 'S' (secret seed) never belong in these commands.
        let mut muxed = VALID_ACCOUNT.to_string();
        muxed.replace_range(0..1, "M");
        assert!(validate_address("attester", &muxed).is_err());

        let mut seed = VALID_ACCOUNT.to_string();
        seed.replace_range(0..1, "S");
        assert!(validate_address("attester", &seed).is_err());
    }

    #[test]
    fn rejects_single_character_typo_via_checksum() {
        // Swap two interior characters: right charset, right length, bad checksum.
        let mut typo: Vec<char> = VALID_ACCOUNT.chars().collect();
        typo.swap(20, 21);
        let typo: String = typo.into_iter().collect();
        assert_ne!(typo, VALID_ACCOUNT);
        assert_eq!(
            validate_address("attester", &typo).unwrap_err(),
            ValidationError::Checksum { field: "attester" }
        );
    }

    #[test]
    fn accepts_valid_record_hash_in_either_case() {
        let lower = "a".repeat(64);
        let mixed = "0F8A".repeat(16);
        assert!(validate_record_hash("record_hash", &lower).is_ok());
        assert!(validate_record_hash("record_hash", &mixed).is_ok());
    }

    #[test]
    fn rejects_malformed_record_hash() {
        assert_eq!(
            validate_record_hash("record_hash", "").unwrap_err(),
            ValidationError::Empty {
                field: "record_hash"
            }
        );
        assert!(matches!(
            validate_record_hash("record_hash", &"ab".repeat(20)),
            Err(ValidationError::Length {
                expected: 64,
                actual: 40,
                ..
            })
        ));
        assert!(matches!(
            validate_record_hash("record_hash", &format!("0x{}", "a".repeat(62))),
            Err(ValidationError::Charset { character: 'x', .. })
        ));
    }

    #[test]
    fn validates_network_names() {
        for ok in [
            "local",
            "standalone",
            "testnet",
            "futurenet",
            "mainnet",
            "my-net_2",
        ] {
            assert!(validate_network_name(ok).is_ok(), "{ok} should be valid");
        }
        assert_eq!(
            validate_network_name("").unwrap_err(),
            ValidationError::Empty { field: "network" }
        );
        assert!(matches!(
            validate_network_name("test net"),
            Err(ValidationError::Charset { character: ' ', .. })
        ));
        assert!(matches!(
            validate_network_name("../../etc/passwd"),
            Err(ValidationError::Charset { .. })
        ));
        assert!(matches!(
            validate_network_name(&"n".repeat(33)),
            Err(ValidationError::TooLong { max: 32, .. })
        ));
    }

    #[test]
    fn validates_source_accounts() {
        assert!(validate_source_account("deployer").is_ok());
        assert!(validate_source_account("admin.testnet_1").is_ok());
        assert!(validate_source_account(VALID_ACCOUNT).is_ok());

        assert_eq!(
            validate_source_account("").unwrap_err(),
            ValidationError::Empty { field: "source" }
        );
        assert!(matches!(
            validate_source_account("admin; rm -rf /"),
            Err(ValidationError::Charset { .. })
        ));
        // G-prefixed values of strkey length are held to the full strkey check.
        let mut typo = VALID_ACCOUNT.to_string();
        typo.replace_range(55..56, "A");
        assert!(matches!(
            validate_source_account(&typo),
            Err(ValidationError::Checksum { .. })
        ));
    }

    #[test]
    fn validates_rpc_urls() {
        assert!(validate_rpc_url("rpc_url", "https://soroban-testnet.stellar.org").is_ok());
        assert!(validate_rpc_url("rpc_url", "http://localhost:8000/soroban/rpc").is_ok());
        assert!(matches!(
            validate_rpc_url("rpc_url", ""),
            Err(ValidationError::Empty { .. })
        ));
        assert!(matches!(
            validate_rpc_url("rpc_url", "ftp://example.org"),
            Err(ValidationError::RpcUrl { .. })
        ));
        assert!(matches!(
            validate_rpc_url("rpc_url", "https://"),
            Err(ValidationError::RpcUrl { .. })
        ));
    }

    #[test]
    fn drift_fields_cover_all_validators() {
        // Contract ids and wiring share the contract strkey check.
        assert!(DriftField::ContractId
            .validate("attester_registry", VALID_CONTRACT)
            .is_ok());
        assert!(DriftField::Wiring
            .validate("attestation_registry", VALID_CONTRACT_2)
            .is_ok());
        assert!(DriftField::Admin
            .validate("admin", VALID_ACCOUNT)
            .is_ok());
        assert!(DriftField::WasmHash
            .validate("wasm_hash", &"ab".repeat(32))
            .is_ok());

        // Mismatched kinds fail closed with the same errors as the validators.
        assert!(matches!(
            DriftField::Admin.validate("admin", VALID_CONTRACT),
            Err(ValidationError::Prefix { expected: "G", .. })
        ));
        assert!(matches!(
            DriftField::ContractId.validate("attester_registry", VALID_ACCOUNT),
            Err(ValidationError::Prefix { expected: "C", .. })
        ));
        assert!(matches!(
            DriftField::WasmHash.validate("wasm_hash", "not-hex"),
            Err(ValidationError::Length { .. })
        ));
    }

    #[test]
    fn drift_fields_are_stable_and_labelled() {
        assert_eq!(DRIFT_FIELDS.len(), 4);
        for field in DRIFT_FIELDS {
            assert!(!field.label().is_empty());
        }
    }

    #[test]
    fn classifies_network_profiles() {
        assert_eq!(NetworkClass::from_profile("mainnet"), NetworkClass::Mainnet);
        assert_eq!(NetworkClass::from_profile("public"), NetworkClass::Mainnet);
        assert_eq!(NetworkClass::from_profile("local"), NetworkClass::Local);
        assert_eq!(NetworkClass::from_profile("standalone"), NetworkClass::Local);
        assert_eq!(NetworkClass::from_profile("testnet"), NetworkClass::Testnet);
        assert_eq!(NetworkClass::from_profile("futurenet"), NetworkClass::Testnet);
        assert_eq!(NetworkClass::from_profile("unknown"), NetworkClass::Testnet);

        assert!(NetworkClass::Mainnet.requires_confirmation());
        assert!(!NetworkClass::Testnet.requires_confirmation());
        assert!(!NetworkClass::Local.requires_confirmation());
    }

    #[test]
    fn verifies_profile_passphrase_before_signing() {
        assert!(validate_profile_passphrase("mainnet", PASSPHRASE_MAINNET).is_ok());
        assert!(validate_profile_passphrase("local", PASSPHRASE_LOCAL).is_ok());
        assert!(validate_profile_passphrase("testnet", PASSPHRASE_TESTNET).is_ok());

        assert!(matches!(
            validate_profile_passphrase("mainnet", PASSPHRASE_TESTNET),
            Err(ValidationError::PassphraseMismatch { profile: "mainnet", .. })
        ));
        assert!(matches!(
            validate_profile_passphrase("local", PASSPHRASE_MAINNET),
            Err(ValidationError::PassphraseMismatch { profile: "local", .. })
        ));
        // Malformed passphrases are rejected before any comparison.
        assert!(matches!(
            validate_profile_passphrase("testnet", "bad\npassphrase"),
            Err(ValidationError::Charset { .. })
        ));
    }

    #[test]
    fn rejects_mainnet_profile_with_testnet_contracts() {
        assert!(validate_profile_deployment("mainnet", "mainnet").is_ok());
        assert!(matches!(
            validate_profile_deployment("mainnet", "testnet"),
            Err(ValidationError::MixedNetworkConfig {
                profile: "mainnet",
                field: "contract ids",
                found: "testnet",
            })
        ));
        assert!(matches!(
            validate_profile_deployment("mainnet", "local"),
            Err(ValidationError::MixedNetworkConfig { found: "local", .. })
        ));
        // Non-mainnet profiles are never blocked by this check.
        assert!(validate_profile_deployment("testnet", "local").is_ok());
        assert!(validate_profile_deployment("local", "testnet").is_ok());
    }

    #[test]
    fn crc16_matches_known_vector() {
        // CRC16-XModem of "123456789" is 0x31C3.
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn base32_matches_known_vector() {
        // RFC 4648 test vector: "foobar" -> MZXW6YTBOI (padding stripped).
        assert_eq!(decode_base32("MZXW6YTBOI").unwrap()[..6], b"foobar"[..]);
        assert!(decode_base32("mzxw").is_none());
    }

    #[test]
    fn error_messages_are_actionable_and_non_sensitive() {
        let err = validate_address("attester", "GABC")
            .unwrap_err()
            .to_string();
        assert!(err.contains("attester"), "{err}");
        assert!(err.contains("56"), "{err}");
        assert!(
            !err.contains("GABC"),
            "addresses are not echoed back: {err}"
        );
    }
}
