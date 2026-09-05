//! Shared loader for `config/networks.toml`
//! Used by deploy script (via Rust wrapper) and admin CLI.
//! No secrets are ever stored in the config file — only public RPC URLs,
//! passphrases, and contract IDs.

pub mod validation;

use serde::Deserialize;
use std::{collections::BTreeMap, fmt, fs, path::Path, path::PathBuf};
use thiserror::Error;

pub use validation::{
    validate_account_address, validate_address, validate_contract_id, validate_network_name,
    validate_record_hash, validate_rpc_url, validate_source_account, AddressKind, ValidationError,
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config/networks.toml not found at {path}")]
    NotFound(PathBuf),
    #[error("failed to read config {path}: {source}")]
    ReadError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse TOML {path}: {source}")]
    ParseError {
        path: PathBuf,
        // Boxed to keep `ConfigError` small: it is returned from the validation
        // helpers on every command path.
        source: Box<toml::de::Error>,
    },
    #[error("network '{0}' not found. Available: {1}")]
    NetworkNotFound(String, String),
    #[error("invalid network name: {0}")]
    InvalidNetworkName(#[source] ValidationError),
    #[error("network '{network}' has an invalid {field}: {source}")]
    InvalidNetworkConfig {
        network: String,
        field: &'static str,
        #[source]
        source: ValidationError,
    },
    #[error("{0}")]
    NotDeployed(String),
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

/// Which contract a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractKind {
    AttesterRegistry,
    AttestationRegistry,
}

impl ContractKind {
    /// Key name as it appears under `[<network>.contracts]`.
    pub fn key(self) -> &'static str {
        match self {
            ContractKind::AttesterRegistry => "attester_registry",
            ContractKind::AttestationRegistry => "attestation_registry",
        }
    }
}

impl fmt::Display for ContractKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// How complete the deployment of a network profile is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentState {
    /// No contract ids recorded yet.
    NotDeployed,
    /// Some, but not all, contract ids are recorded.
    Partial { missing: Vec<ContractKind> },
    /// Every contract id is recorded.
    Deployed,
}

impl NetworkConfig {
    pub fn is_deployed(&self) -> bool {
        !self.contracts.attester_registry.is_empty()
            && !self.contracts.attestation_registry.is_empty()
    }

    /// Contract id for `kind`, or an empty string when it has not been deployed.
    pub fn contract_id(&self, kind: ContractKind) -> &str {
        match kind {
            ContractKind::AttesterRegistry => &self.contracts.attester_registry,
            ContractKind::AttestationRegistry => &self.contracts.attestation_registry,
        }
    }

    /// Classify the deployment profile so partial states can be reported clearly.
    pub fn deployment_state(&self) -> DeploymentState {
        let missing: Vec<ContractKind> = [
            ContractKind::AttesterRegistry,
            ContractKind::AttestationRegistry,
        ]
        .into_iter()
        .filter(|kind| self.contract_id(*kind).is_empty())
        .collect();

        match missing.len() {
            0 => DeploymentState::Deployed,
            2 => DeploymentState::NotDeployed,
            _ => DeploymentState::Partial { missing },
        }
    }

    /// Validate the public config values of this network profile.
    ///
    /// Empty contract ids are accepted (the network is simply not deployed yet);
    /// non-empty ones must be well-formed `C...` strkeys.
    pub fn validate(&self, network: &str) -> Result<(), ConfigError> {
        let invalid =
            |field: &'static str, source: ValidationError| ConfigError::InvalidNetworkConfig {
                network: network.to_string(),
                field,
                source,
            };

        validate_rpc_url("rpc_url", &self.rpc_url).map_err(|e| invalid("rpc_url", e))?;
        if self.network_passphrase.trim().is_empty() {
            return Err(invalid(
                "network_passphrase",
                ValidationError::Empty {
                    field: "network_passphrase",
                },
            ));
        }

        for kind in [
            ContractKind::AttesterRegistry,
            ContractKind::AttestationRegistry,
        ] {
            let id = self.contract_id(kind);
            if !id.is_empty() {
                validate_contract_id(kind.key(), id).map_err(|e| invalid(kind.key(), e))?;
            }
        }

        Ok(())
    }

    /// Resolve a contract id for a command that needs it, reporting partial
    /// deployments in a way an operator can act on.
    pub fn require_contract_id(
        &self,
        network: &str,
        kind: ContractKind,
    ) -> Result<&str, ConfigError> {
        let id = self.contract_id(kind);
        if id.is_empty() {
            let detail = match self.deployment_state() {
                DeploymentState::Partial { .. } => format!(
                    "network '{network}' is partially deployed: {} is set but {} is missing",
                    match kind {
                        ContractKind::AttesterRegistry => ContractKind::AttestationRegistry,
                        ContractKind::AttestationRegistry => ContractKind::AttesterRegistry,
                    },
                    kind
                ),
                _ => format!("{kind} is not deployed for network '{network}'"),
            };
            return Err(ConfigError::NotDeployed(format!(
                "{detail}. Deploy first: ./scripts/deploy.sh --network {network}, then record the contract id under [{network}.contracts] in config/networks.toml"
            )));
        }
        validate_contract_id(kind.key(), id).map_err(|source| {
            ConfigError::InvalidNetworkConfig {
                network: network.to_string(),
                field: kind.key(),
                source,
            }
        })?;
        Ok(id)
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
        source: Box::new(e),
    })?;

    Ok(networks)
}

pub fn get_network(networks: &Networks, name: &str) -> Result<NetworkConfig, ConfigError> {
    validate_network_name(name).map_err(ConfigError::InvalidNetworkName)?;
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

    const ATTESTER_ID: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const ATTESTATION_ID: &str = "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX";

    fn network(attester: &str, attestation: &str) -> NetworkConfig {
        NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            contracts: ContractIds {
                attester_registry: attester.to_string(),
                attestation_registry: attestation.to_string(),
            },
        }
    }

    #[test]
    fn rejects_malformed_network_name() {
        let networks = Networks::new();
        assert!(matches!(
            get_network(&networks, ""),
            Err(ConfigError::InvalidNetworkName(_))
        ));
        assert!(matches!(
            get_network(&networks, "../secrets"),
            Err(ConfigError::InvalidNetworkName(_))
        ));
    }

    #[test]
    fn deployment_state_covers_empty_partial_and_complete() {
        assert_eq!(
            network("", "").deployment_state(),
            DeploymentState::NotDeployed
        );
        assert_eq!(
            network(ATTESTER_ID, "").deployment_state(),
            DeploymentState::Partial {
                missing: vec![ContractKind::AttestationRegistry]
            }
        );
        assert_eq!(
            network("", ATTESTATION_ID).deployment_state(),
            DeploymentState::Partial {
                missing: vec![ContractKind::AttesterRegistry]
            }
        );
        assert_eq!(
            network(ATTESTER_ID, ATTESTATION_ID).deployment_state(),
            DeploymentState::Deployed
        );
    }

    #[test]
    fn require_contract_id_reports_partial_deployment() {
        let cfg = network(ATTESTER_ID, "");
        assert_eq!(
            cfg.require_contract_id("testnet", ContractKind::AttesterRegistry)
                .unwrap(),
            ATTESTER_ID
        );

        let err = cfg
            .require_contract_id("testnet", ContractKind::AttestationRegistry)
            .unwrap_err()
            .to_string();
        assert!(err.contains("partially deployed"), "{err}");
        assert!(err.contains("attestation_registry"), "{err}");
        assert!(err.contains("--network testnet"), "{err}");
    }

    #[test]
    fn require_contract_id_reports_undeployed_network() {
        let err = network("", "")
            .require_contract_id("local", ContractKind::AttesterRegistry)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not deployed"), "{err}");
        assert!(!err.contains("partially"), "{err}");
    }

    #[test]
    fn require_contract_id_rejects_malformed_id() {
        let err = network("CA6P...", ATTESTATION_ID)
            .require_contract_id("testnet", ContractKind::AttesterRegistry)
            .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidNetworkConfig {
                field: "attester_registry",
                ..
            }
        ));
    }

    #[test]
    fn validate_accepts_undeployed_profile_and_rejects_bad_values() {
        assert!(network("", "").validate("local").is_ok());
        assert!(network(ATTESTER_ID, ATTESTATION_ID)
            .validate("testnet")
            .is_ok());

        let mut bad = network(ATTESTER_ID, ATTESTATION_ID);
        bad.rpc_url = "soroban-testnet.stellar.org".to_string();
        assert!(matches!(
            bad.validate("testnet"),
            Err(ConfigError::InvalidNetworkConfig {
                field: "rpc_url",
                ..
            })
        ));

        let mut bad = network(ATTESTER_ID, ATTESTATION_ID);
        bad.network_passphrase = "   ".to_string();
        assert!(matches!(
            bad.validate("testnet"),
            Err(ConfigError::InvalidNetworkConfig {
                field: "network_passphrase",
                ..
            })
        ));

        assert!(matches!(
            network(ATTESTER_ID, "CBAD").validate("testnet"),
            Err(ConfigError::InvalidNetworkConfig {
                field: "attestation_registry",
                ..
            })
        ));
    }

    #[test]
    fn shipped_config_is_valid() {
        // The committed config/networks.toml must always parse and validate.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/networks.toml");
        let networks = load_networks(Some(&path)).unwrap();
        assert!(!networks.is_empty());
        for (name, cfg) in &networks {
            validate_network_name(name).unwrap_or_else(|e| panic!("network '{name}': {e}"));
            cfg.validate(name)
                .unwrap_or_else(|e| panic!("network '{name}': {e}"));
        }
    }

    #[test]
    fn missing_config_error_includes_path() {
        let missing = PathBuf::from("/tmp/lafiya-missing-config/networks.toml");
        let err = load_networks::<PathBuf>(Some(&missing)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("config/networks.toml not found at"));
        assert!(msg.contains(&missing.to_string_lossy().to_string()));
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
