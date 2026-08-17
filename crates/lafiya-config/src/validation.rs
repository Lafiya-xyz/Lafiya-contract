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
