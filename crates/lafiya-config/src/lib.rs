//! Shared loader for `config/networks.toml`
//! Used by deploy script (via Rust wrapper) and admin CLI.
//! No secrets are ever stored in the config file — only public RPC URLs,
//! passphrases, and contract IDs.

use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    NotFound(PathBuf),
    #[error("failed to read config {path}: {source}")]
    ReadError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse TOML {path}: {source}")]
    ParseError {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("network '{0}' not found. Available: {1}")]
    NetworkNotFound(String, String),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContractIds {
    #[serde(default)]
    pub attester_registry: String,
    #[serde(default)]
    pub attestation_registry: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    pub rpc_url: String,
    pub network_passphrase: String,
    #[serde(default)]
    pub contracts: ContractIds,
}

impl NetworkConfig {
    pub fn is_deployed(&self) -> bool {
        !self.contracts.attester_registry.is_empty()
            && !self.contracts.attestation_registry.is_empty()
    }
}

pub type Networks = BTreeMap<String, NetworkConfig>;

/// Default path resolution: try current dir config/networks.toml, then parent vyhled up to 3 levels,
/// and finally relative to this crate if used in repo.
pub fn default_config_path() -> PathBuf {
    // Try to find from current working directory
    let candidates = [
        PathBuf::from("config/networks.toml"),
        PathBuf::from("../config/networks.toml"),
        PathBuf::from("../../config/networks.toml"),
        PathBuf::from("../../../config/networks.toml"),
    ];

    for p in candidates {
        if p.exists() {
            return p;
        }
    }

    // Fallback: relative to this crate's manifest dir (if running via cargo from workspace)
    // crates/lafiya-config -> ../../config/networks.toml
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fallback = manifest_dir.join("../../config/networks.toml");
    if fallback.exists() {
        return fallback;
    }

    // Final fallback: just config/networks.toml
    PathBuf::from("config/networks.toml")
}

pub fn load_networks<P: AsRef<Path>>(path: Option<P>) -> Result<Networks, ConfigError> {
    let config_path = match path {
        Some(p) => p.as_ref().to_path_buf(),
        None => default_config_path(),
    };

    if !config_path.exists() {
        return Err(ConfigError::NotFound(config_path));
    }

    let content = fs::read_to_string(&config_path).map_err(|e| ConfigError::ReadError {
        path: config_path.clone(),
        source: e,
    })?;

    let networks: Networks = toml::from_str(&content).map_err(|e| ConfigError::ParseError {
        path: config_path.clone(),
        source: e,
    })?;

    Ok(networks)
}

pub fn get_network(networks: &Networks, name: &str) -> Result<NetworkConfig, ConfigError> {
    networks.get(name).cloned().ok_or_else(|| {
        let available = networks.keys().cloned().collect::<Vec<_>>().join(", ");
        ConfigError::NetworkNotFound(name.to_string(), available)
    })
}

pub fn load_network_config<P: AsRef<Path>>(
    network: &str,
    path: Option<P>,
) -> Result<(PathBuf, NetworkConfig), ConfigError> {
    let config_path = path
        .map(|p| p.as_ref().to_path_buf())
        .unwrap_or_else(default_config_path);

    let networks = load_networks(Some(&config_path))?;
    let cfg = get_network(&networks, network)?;
    Ok((config_path, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_toml() -> &'static str {
        r#"
[local]
rpc_url = "http://localhost:8000/soroban/rpc"
network_passphrase = "Standalone Network ; February 2017"

[local.contracts]
attester_registry = ""
attestation_registry = ""

[testnet]
rpc_url = "https://soroban-testnet.stellar.org"
network_passphrase = "Test SDF Network ; September 2015"

[testnet.contracts]
attester_registry = "CA6P..."
attestation_registry = "CB2X..."

[futurenet]
rpc_url = "https://rpc-futurenet.stellar.org"
network_passphrase = "Test SDF Future Network ; October 2022"

[futurenet.contracts]
attester_registry = ""
attestation_registry = ""

[mainnet]
rpc_url = "https://mainnet.sorobanrpc.com"
network_passphrase = "Public Global Stellar Network ; September 2015"

[mainnet.contracts]
attester_registry = ""
attestation_registry = ""
"#
    }

    #[test]
    fn parses_sample() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(sample_toml().as_bytes()).unwrap();
        let networks = load_networks(Some(file.path())).unwrap();
        assert_eq!(networks.len(), 4);
        let testnet = get_network(&networks, "testnet").unwrap();
        assert_eq!(testnet.rpc_url, "https://soroban-testnet.stellar.org");
        assert_eq!(
            testnet.network_passphrase,
            "Test SDF Network ; September 2015"
        );
        assert_eq!(testnet.contracts.attester_registry, "CA6P...");
        assert!(testnet.is_deployed());

        let local = get_network(&networks, "local").unwrap();
        assert!(!local.is_deployed());
        assert!(local.contracts.attester_registry.is_empty());
    }

    #[test]
    fn missing_network_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(sample_toml().as_bytes()).unwrap();
        let networks = load_networks(Some(file.path())).unwrap();
        let err = get_network(&networks, "nonexistent").unwrap_err();
        match err {
            ConfigError::NetworkNotFound(name, avail) => {
                assert_eq!(name, "nonexistent");
                assert!(avail.contains("testnet"));
            }
            _ => panic!("wrong error"),
        }
    }

    #[test]
    fn secrets_not_in_config_struct() {
        // Ensure our struct does not have fields that could hold secrets
        // This is a compile-time guarantee: we only have rpc_url, passphrase, contracts.
        // No private_key, secret, mnemonic fields.
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(sample_toml().as_bytes()).unwrap();
        let networks = load_networks(Some(file.path())).unwrap();
        for (_, cfg) in networks {
            let debug = format!("{:?}", cfg);
            assert!(!debug.to_lowercase().contains("secret"));
            assert!(!debug.to_lowercase().contains("private_key"));
            assert!(!debug.to_lowercase().contains("mnemonic"));
        }
    }
}
