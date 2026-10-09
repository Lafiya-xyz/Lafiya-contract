//! Strict key checking and layered overrides for `config/networks.toml`.
//!
//! Precedence, highest first:
//!
//! 1. CLI flag (`--set <key>=<value>`)
//! 2. `LAFIYA_<NETWORK>_<KEY>` environment variable
//! 3. `config/networks.local.toml` (gitignored, next to `networks.toml`)
//! 4. `config/networks.toml`
//!
//! [`resolve_network`] records which layer supplied each value so
//! `config show` can report it.
//!
//! `rpc_urls` (the failover list) can be set in either file. When `rpc_url`
//! is overridden by a higher-precedence layer than the one that set
//! `rpc_urls`, the override wins and the list is dropped, so e.g.
//! `LAFIYA_TESTNET_RPC_URL` always takes effect.

use crate::{default_config_path, get_network, load_networks, ConfigError, NetworkConfig};
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

/// Keys allowed in a `[<network>]` table.
pub const NETWORK_KEYS: &[&str] = &["rpc_url", "rpc_urls", "network_passphrase", "contracts"];
/// Keys allowed in a `[<network>.contracts]` table.
pub const CONTRACT_KEYS: &[&str] = &[
    "attester_registry",
    "attestation_registry",
    "incentive_pool",
];
/// Values that can be overridden per invocation, as dotted paths within a network.
pub const OVERRIDE_KEYS: &[&str] = &[
    "rpc_url",
    "network_passphrase",
    "contracts.attester_registry",
    "contracts.attestation_registry",
];

/// Where a resolved value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    LocalFile(PathBuf),
    Env(String),
    Flag,
}

impl Source {
    fn precedence(&self) -> u8 {
        match self {
            Source::File(_) => 0,
            Source::LocalFile(_) => 1,
            Source::Env(_) => 2,
            Source::Flag => 3,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::File(p) => write!(f, "file {}", p.display()),
            Source::LocalFile(p) => write!(f, "local file {}", p.display()),
            Source::Env(var) => write!(f, "env {var}"),
            Source::Flag => f.write_str("--set flag"),
        }
    }
}

/// A network profile after all override layers were applied.
#[derive(Debug, Clone)]
pub struct ResolvedNetwork {
    pub config_path: PathBuf,
    pub config: NetworkConfig,
    /// Source of every key in [`OVERRIDE_KEYS`], plus `rpc_urls`.
    pub sources: BTreeMap<&'static str, Source>,
}

/// Closest candidate within Levenshtein distance 2, if any.
pub fn suggest<'a>(key: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (levenshtein(key, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn unknown_key(location: &str, path: String, key: &str, valid: &[&str]) -> ConfigError {
    ConfigError::UnknownKey {
        location: location.to_string(),
        key: path,
        hint: suggest(key, valid)
            .map(|s| format!(" (did you mean `{s}`?)"))
            .unwrap_or_default(),
    }
}

/// Reject any key a network profile does not define, naming the closest valid key.
pub fn check_unknown_keys(location: &str, table: &toml::Table) -> Result<(), ConfigError> {
    for (network, value) in table {
        // Non-table values are reported by serde with its own type error.
        let Some(net) = value.as_table() else {
            continue;
        };
        for (key, value) in net {
            if !NETWORK_KEYS.contains(&key.as_str()) {
                return Err(unknown_key(
                    location,
                    format!("{network}.{key}"),
                    key,
                    NETWORK_KEYS,
                ));
            }
            if let (true, Some(contracts)) = (key == "contracts", value.as_table()) {
                for key in contracts.keys() {
                    if !CONTRACT_KEYS.contains(&key.as_str()) {
                        return Err(unknown_key(
                            location,
                            format!("{network}.contracts.{key}"),
                            key,
                            CONTRACT_KEYS,
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// `config/networks.local.toml` next to the given `networks.toml`.
pub fn local_override_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("networks.local.toml")
}

/// Environment variable overriding `key` (an [`OVERRIDE_KEYS`] entry) for `network`,
/// e.g. `LAFIYA_TESTNET_RPC_URL` or `LAFIYA_TESTNET_ATTESTER_REGISTRY`.
pub fn env_var_name(network: &str, key: &str) -> String {
    format!(
        "LAFIYA_{}_{}",
        network.to_uppercase().replace('-', "_"),
        key.trim_start_matches("contracts.").to_uppercase()
    )
}

fn set(cfg: &mut NetworkConfig, key: &str, value: String) {
    match key {
        "rpc_url" => cfg.rpc_url = value,
        "network_passphrase" => cfg.network_passphrase = value,
        "contracts.attester_registry" => cfg.contracts.attester_registry = value,
        "contracts.attestation_registry" => cfg.contracts.attestation_registry = value,
        _ => unreachable!("not an override key: {key}"),
    }
}

fn lookup<'a>(table: &'a toml::Table, dotted: &str) -> Option<&'a toml::Value> {
    match dotted.split_once('.') {
        Some((head, rest)) => lookup(table.get(head)?.as_table()?, rest),
        None => table.get(dotted),
    }
}

/// Resolve `network` through every override layer (see the module docs).
///
/// `env` looks up an environment variable (pass `|k| std::env::var(k).ok()`);
/// `flags` are `(key, value)` pairs from `--set`, applied in order.
pub fn resolve_network(
    network: &str,
    config_path: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
    flags: &[(String, String)],
) -> Result<ResolvedNetwork, ConfigError> {
    let config_path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);
    let networks = load_networks(Some(&config_path))?;
    let mut config = get_network(&networks, network)?;
    let mut sources: BTreeMap<&'static str, Source> = OVERRIDE_KEYS
        .iter()
        .chain(["rpc_urls"].iter())
        .map(|k| (*k, Source::File(config_path.clone())))
        .collect();

    let local = local_override_path(&config_path);
    if local.exists() {
        let location = local.display().to_string();
        let content = fs::read_to_string(&local).map_err(|source| ConfigError::ReadError {
            path: local.clone(),
            source,
        })?;
        let table: toml::Table = toml::from_str(&content).map_err(|e| ConfigError::ParseError {
            path: local.clone(),
            source: Box::new(e),
        })?;
        check_unknown_keys(&location, &table)?;
        if let Some(net) = table.get(network).and_then(toml::Value::as_table) {
            if let Some(urls) = net.get("rpc_urls") {
                let not_strings = || ConfigError::OverrideNotString {
                    location: location.clone(),
                    key: format!("{network}.rpc_urls"),
                };
                config.rpc_urls = urls
                    .as_array()
                    .ok_or_else(not_strings)?
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or_else(not_strings))
                    .collect::<Result<_, _>>()?;
                sources.insert("rpc_urls", Source::LocalFile(local.clone()));
            }
            for key in OVERRIDE_KEYS {
                if let Some(value) = lookup(net, key) {
                    let value = value
                        .as_str()
                        .ok_or_else(|| ConfigError::OverrideNotString {
                            location: location.clone(),
                            key: format!("{network}.{key}"),
                        })?;
                    set(&mut config, key, value.to_string());
                    sources.insert(key, Source::LocalFile(local.clone()));
                }
            }
        }
    }

    for key in OVERRIDE_KEYS {
        let var = env_var_name(network, key);
        if let Some(value) = env(&var) {
            set(&mut config, key, value);
            sources.insert(key, Source::Env(var));
        }
    }

    for (key, value) in flags {
        let key = OVERRIDE_KEYS
            .iter()
            .find(|k| **k == key)
            .ok_or_else(|| unknown_key("--set", key.clone(), key, OVERRIDE_KEYS))?;
        set(&mut config, key, value.clone());
        sources.insert(key, Source::Flag);
    }

    if sources["rpc_url"].precedence() > sources["rpc_urls"].precedence() {
        config.rpc_urls.clear();
        sources.insert("rpc_urls", sources["rpc_url"].clone());
    }

    Ok(ResolvedNetwork {
        config_path,
        config,
        sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[testnet]
rpc_url = "https://file.example"
network_passphrase = "Test SDF Network ; September 2015"

[testnet.contracts]
attester_registry = ""
attestation_registry = ""
"#;

    fn setup(local: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("networks.toml"), BASE).unwrap();
        if let Some(local) = local {
            fs::write(dir.path().join("networks.local.toml"), local).unwrap();
        }
        dir
    }

    fn resolve(
        dir: &tempfile::TempDir,
        env: &[(&str, &str)],
        flags: &[(&str, &str)],
    ) -> Result<ResolvedNetwork, ConfigError> {
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let flags: Vec<(String, String)> = flags
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        resolve_network(
            "testnet",
            Some(&dir.path().join("networks.toml")),
            |k| env.get(k).cloned(),
            &flags,
        )
    }

    const LOCAL: &str = "[testnet]\nrpc_url = \"https://local.example\"\n";
    const ENV: (&str, &str) = ("LAFIYA_TESTNET_RPC_URL", "https://env.example");
    const FLAG: (&str, &str) = ("rpc_url", "https://flag.example");

    #[test]
    fn file_only() {
        let dir = setup(None);
        let r = resolve(&dir, &[], &[]).unwrap();
        assert_eq!(r.config.rpc_url, "https://file.example");
        assert!(matches!(r.sources["rpc_url"], Source::File(_)));
    }

    #[test]
    fn local_file_beats_file() {
        let dir = setup(Some(LOCAL));
        let r = resolve(&dir, &[], &[]).unwrap();
        assert_eq!(r.config.rpc_url, "https://local.example");
        assert!(matches!(r.sources["rpc_url"], Source::LocalFile(_)));
        // Keys the local file does not set still come from networks.toml.
        assert!(matches!(r.sources["network_passphrase"], Source::File(_)));
    }

    #[test]
    fn env_beats_local_file() {
        let dir = setup(Some(LOCAL));
        let r = resolve(&dir, &[ENV], &[]).unwrap();
        assert_eq!(r.config.rpc_url, "https://env.example");
        assert_eq!(
            r.sources["rpc_url"],
            Source::Env("LAFIYA_TESTNET_RPC_URL".into())
        );
    }

    #[test]
    fn flag_beats_env() {
        let dir = setup(Some(LOCAL));
        let r = resolve(&dir, &[ENV], &[FLAG]).unwrap();
        assert_eq!(r.config.rpc_url, "https://flag.example");
        assert_eq!(r.sources["rpc_url"], Source::Flag);
    }

    #[test]
    fn rpc_url_override_replaces_file_rpc_urls() {
        let dir = setup(None);
        fs::write(
            dir.path().join("networks.toml"),
            BASE.replace(
                "[testnet.contracts]",
                "rpc_urls = [\"https://a.example\", \"https://b.example\"]\n\n[testnet.contracts]",
            ),
        )
        .unwrap();
        let r = resolve(&dir, &[], &[]).unwrap();
        assert_eq!(
            r.config.rpc_endpoints(),
            vec!["https://a.example", "https://b.example"]
        );

        let r = resolve(&dir, &[ENV], &[]).unwrap();
        assert_eq!(r.config.rpc_endpoints(), vec!["https://env.example"]);
    }

    #[test]
    fn local_file_can_set_rpc_urls() {
        let dir = setup(Some(
            "[testnet]\nrpc_urls = [\"https://l1.example\", \"https://l2.example\"]\n",
        ));
        let r = resolve(&dir, &[], &[]).unwrap();
        assert_eq!(
            r.config.rpc_endpoints(),
            vec!["https://l1.example", "https://l2.example"]
        );
        assert!(matches!(r.sources["rpc_urls"], Source::LocalFile(_)));
    }

    #[test]
    fn contract_ids_can_be_overridden() {
        let dir = setup(None);
        let r = resolve(
            &dir,
            &[("LAFIYA_TESTNET_ATTESTER_REGISTRY", "CENV")],
            &[("contracts.attestation_registry", "CFLAG")],
        )
        .unwrap();
        assert_eq!(r.config.contracts.attester_registry, "CENV");
        assert_eq!(r.config.contracts.attestation_registry, "CFLAG");
    }

    #[test]
    fn unknown_flag_key_is_rejected_with_suggestion() {
        let dir = setup(None);
        let err = resolve(&dir, &[], &[("rpc_ur", "x")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown key `rpc_ur` in --set"), "{err}");
        assert!(err.contains("did you mean `rpc_url`?"), "{err}");
    }

    #[test]
    fn unknown_local_file_key_is_rejected() {
        let dir = setup(Some("[testnet]\nrpc_uri = \"x\"\n"));
        let err = resolve(&dir, &[], &[]).unwrap_err().to_string();
        assert!(err.contains("unknown key `testnet.rpc_uri`"), "{err}");
        assert!(err.contains("networks.local.toml"), "{err}");
    }

    #[test]
    fn env_var_names() {
        assert_eq!(env_var_name("testnet", "rpc_url"), "LAFIYA_TESTNET_RPC_URL");
        assert_eq!(
            env_var_name("my-net", "contracts.attester_registry"),
            "LAFIYA_MY_NET_ATTESTER_REGISTRY"
        );
    }

    #[test]
    fn suggestions() {
        assert_eq!(
            suggest("atester_registry", CONTRACT_KEYS),
            Some("attester_registry")
        );
        assert_eq!(suggest("contract", NETWORK_KEYS), Some("contracts"));
        assert_eq!(suggest("private_key", NETWORK_KEYS), None);
    }
}
