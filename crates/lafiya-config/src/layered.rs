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
    sync::Arc,
};

/// Prefix marking a value as a reference into a secret store.
pub const SECRET_SCHEME: &str = "secret://";

/// A pluggable secret store.
///
/// Providers are looked up by scheme (the part of a `secret://<scheme>/<path>`
/// reference before the first `/`). Implementations must never log or
/// serialize resolved values.
pub trait SecretProvider: fmt::Debug + Send + Sync {
    /// Scheme this provider answers to, e.g. `env` or `file`.
    fn scheme(&self) -> &str;

    /// Resolve `path` (the part after `secret://<scheme>/`) to its secret value.
    fn resolve(&self, path: &str) -> Result<String, ConfigError>;
}

/// Registry of [`SecretProvider`]s used to resolve `secret://` references.
///
/// Unknown schemes fail closed: resolution returns
/// [`ConfigError::UnknownSecretProvider`] rather than passing the reference
/// through as a literal value.
#[derive(Debug, Clone, Default)]
pub struct SecretResolver {
    providers: BTreeMap<String, Arc<dyn SecretProvider>>,
}

impl SecretResolver {
    /// An empty resolver; every `secret://` reference fails closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The default resolver: `env` and `file` providers.
    pub fn with_defaults() -> Self {
        Self::new()
            .with_provider(Arc::new(EnvSecretProvider))
            .with_provider(Arc::new(FileSecretProvider))
    }

    /// Register `provider`, replacing any provider with the same scheme.
    pub fn with_provider(mut self, provider: Arc<dyn SecretProvider>) -> Self {
        self.providers
            .insert(provider.scheme().to_string(), provider);
        self
    }

    /// Resolve `value` if it is a `secret://` reference, otherwise return it unchanged.
    pub fn resolve(&self, value: &str) -> Result<String, ConfigError> {
        let Some(reference) = value.strip_prefix(SECRET_SCHEME) else {
            return Ok(value.to_string());
        };
        let (scheme, path) = reference.split_once('/').ok_or_else(|| {
            ConfigError::MalformedSecretReference {
                reference: value.to_string(),
            }
        })?;
        let provider = self
            .providers
            .get(scheme)
            .ok_or_else(|| ConfigError::UnknownSecretProvider {
                scheme: scheme.to_string(),
                reference: value.to_string(),
            })?;
        provider.resolve(path)
    }
}

/// Reads secrets from environment variables: `secret://env/VAR_NAME`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn scheme(&self) -> &str {
        "env"
    }

    fn resolve(&self, path: &str) -> Result<String, ConfigError> {
        std::env::var(path).map_err(|_| ConfigError::SecretNotFound {
            reference: format!("{SECRET_SCHEME}env/{path}"),
        })
    }
}

/// Reads secrets from files: `secret://file/path/to/secret`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileSecretProvider;

impl SecretProvider for FileSecretProvider {
    fn scheme(&self) -> &str {
        "file"
    }

    fn resolve(&self, path: &str) -> Result<String, ConfigError> {
        let content = fs::read_to_string(path).map_err(|source| ConfigError::ReadError {
            path: PathBuf::from(path),
            source,
        })?;
        Ok(content.trim_end_matches(['\n', '\r']).to_string())
    }
}

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
///
/// `config` holds resolved values and must never be serialized directly when
/// any layer used a `secret://` reference; use [`ResolvedNetwork::redacted`]
/// for anything that leaves the process.
#[derive(Debug, Clone)]
pub struct ResolvedNetwork {
    pub config_path: PathBuf,
    pub config: NetworkConfig,
    /// Source of every key in [`OVERRIDE_KEYS`], plus `rpc_urls`.
    pub sources: BTreeMap<&'static str, Source>,
    /// Keys whose value came from a `secret://` reference.
    pub secret_keys: Vec<&'static str>,
}

impl ResolvedNetwork {
    /// A copy of the config with every secret-derived value replaced by
    /// `secret://<redacted>`, safe to serialize or print.
    pub fn redacted(&self) -> NetworkConfig {
        let mut config = self.config.clone();
        for key in &self.secret_keys {
            match *key {
                "rpc_url" => config.rpc_url = format!("{SECRET_SCHEME}<redacted>"),
                "network_passphrase" => {
                    config.network_passphrase = format!("{SECRET_SCHEME}<redacted>")
                }
                "contracts.attester_registry" => {
                    config.contracts.attester_registry = format!("{SECRET_SCHEME}<redacted>")
                }
                "contracts.attestation_registry" => {
                    config.contracts.attestation_registry = format!("{SECRET_SCHEME}<redacted>")
                }
                _ => {}
            }
        }
        config
    }
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

/// `config/profile.toml` next to the given `networks.toml`.
///
/// Holds the active named profile selected with `lafiya-cli profile use <name>`.
pub fn profile_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("profile.toml")
}

/// Environment variable that overrides the persisted active profile.
pub const PROFILE_ENV_VAR: &str = "LAFIYA_PROFILE";

/// The active named profile, as selected by `lafiya-cli profile use <name>`.
///
/// The profile is a *selection* of a network, not a network definition: it is
/// persisted in `config/profile.toml` and may be overridden per invocation by
/// `--network` or [`PROFILE_ENV_VAR`]. It is always resolved against
/// `config/networks.toml`, so a profile naming an unknown network fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProfile {
    /// Profile (network) name, e.g. `local`, `testnet`, `mainnet`.
    pub name: String,
    /// Where the selection came from.
    pub source: Source,
}

impl ActiveProfile {
    /// Whether this profile targets mainnet, which requires extra guard rails.
    pub fn is_mainnet(&self) -> bool {
        is_mainnet(&self.name)
    }
}

/// Whether `network` names mainnet.
///
/// Matching is case-insensitive and accepts the common spellings `mainnet`,
/// `main-net` and `main_net` so a typo cannot silently bypass guard rails.
pub fn is_mainnet(network: &str) -> bool {
    matches!(
        network.to_ascii_lowercase().replace(['-', '_'], "").as_str(),
        "mainnet" | "pubnet" | "public"
    )
}

/// Read the persisted active profile from `config/profile.toml`.
///
/// Returns `Ok(None)` when the file does not exist. Unknown keys are rejected
/// so a typo cannot silently leave the profile unset.
pub fn load_profile(config_path: &Path) -> Result<Option<ActiveProfile>, ConfigError> {
    let path = profile_path(config_path);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|source| ConfigError::ReadError {
        path: path.clone(),
        source,
    })?;
    let table: toml::Table = toml::from_str(&content).map_err(|e| ConfigError::ParseError {
        path: path.clone(),
        source: Box::new(e),
    })?;
    for key in table.keys() {
        if key != "profile" {
            return Err(unknown_key(
                &path.display().to_string(),
                key.clone(),
                key,
                &["profile"],
            ));
        }
    }
    let Some(value) = table.get("profile") else {
        return Ok(None);
    };
    let name = value
        .as_str()
        .ok_or_else(|| ConfigError::OverrideNotString {
            location: path.display().to_string(),
            key: "profile".to_string(),
        })?
        .trim();
    if name.is_empty() {
        return Err(ConfigError::EmptyProfile {
            path: path.display().to_string(),
        });
    }
    Ok(Some(ActiveProfile {
        name: name.to_string(),
        source: Source::File(path),
    }))
}

/// Persist `name` as the active profile in `config/profile.toml`.
///
/// The profile must name a network defined in `config/networks.toml`; this
/// fails closed rather than writing a selection that cannot be resolved.
pub fn save_profile(config_path: &Path, name: &str) -> Result<ActiveProfile, ConfigError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ConfigError::EmptyProfile {
            path: profile_path(config_path).display().to_string(),
        });
    }
    let networks = load_networks(Some(config_path))?;
    get_network(&networks, name)?;
    let path = profile_path(config_path);
    let content = format!("profile = \"{}\"\n", toml_escape(name));
    fs::write(&path, content).map_err(|source| ConfigError::WriteError {
        path: path.clone(),
        source,
    })?;
    Ok(ActiveProfile {
        name: name.to_string(),
        source: Source::File(path),
    })
}

/// Resolve the active profile for this invocation.
///
/// Precedence, highest first: `--network` flag, [`PROFILE_ENV_VAR`], the
/// persisted `config/profile.toml`. Returns `Ok(None)` when nothing selects a
/// profile, leaving the choice to the caller.
pub fn resolve_profile(
    config_path: &Path,
    flag: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<ActiveProfile>, ConfigError> {
    if let Some(name) = flag {
        let name = name.trim();
        if name.is_empty() {
            return Err(ConfigError::EmptyProfile {
                path: "--network flag".to_string(),
            });
        }
        return Ok(Some(ActiveProfile {
            name: name.to_string(),
            source: Source::Flag,
        }));
    }
    if let Some(name) = env(PROFILE_ENV_VAR) {
        let name = name.trim();
        if name.is_empty() {
            return Err(ConfigError::EmptyProfile {
                path: format!("env {PROFILE_ENV_VAR}"),
            });
        }
        return Ok(Some(ActiveProfile {
            name: name.to_string(),
            source: Source::Env(PROFILE_ENV_VAR.to_string()),
        }));
    }
    load_profile(config_path)
}

/// Escape a value for inclusion in a double-quoted TOML basic string.
fn toml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
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
///
/// Any value (from any layer) of the form `secret://<scheme>/<path>` is
/// resolved through `secrets`. Unknown schemes fail closed.
pub fn resolve_network(
    network: &str,
    config_path: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
    flags: &[(String, String)],
    secrets: &SecretResolver,
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
    let mut secret_keys: Vec<&'static str> = Vec::new();

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
                    let resolved = secrets.resolve(value)?;
                    if value.starts_with(SECRET_SCHEME) {
                        secret_keys.push(key);
                    }
                    set(&mut config, key, resolved);
                    sources.insert(key, Source::LocalFile(local.clone()));
                }
            }
        }
    }

    for key in OVERRIDE_KEYS {
        let var = env_var_name(network, key);
        if let Some(value) = env(&var) {
            let resolved = secrets.resolve(&value)?;
            if value.starts_with(SECRET_SCHEME) {
                secret_keys.push(key);
            }
            set(&mut config, key, resolved);
            sources.insert(key, Source::Env(var));
        }
    }

    for (key, value) in flags {
        let key = OVERRIDE_KEYS
            .iter()
            .find(|k| **k == key)
            .ok_or_else(|| unknown_key("--set", key.clone(), key, OVERRIDE_KEYS))?;
        let resolved = secrets.resolve(value)?;
        if value.starts_with(SECRET_SCHEME) {
            secret_keys.push(key);
        }
        set(&mut config, key, resolved);
        sources.insert(key, Source::Flag);
    }

    if sources["rpc_url"].precedence() > sources["rpc_urls"].precedence() {
        config.rpc_urls.clear();
        sources.insert("rpc_urls", sources["rpc_url"].clone());
    }

    secret_keys.sort_unstable();
    secret_keys.dedup();

    Ok(ResolvedNetwork {
        config_path,
        config,
        sources,
        secret_keys,
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
            &SecretResolver::with_defaults(),
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

    #[test]
    fn secret_reference_resolved_and_redacted() {
        let dir = setup(Some(
            "[testnet]\nrpc_url = \"secret://env/MY_RPC\"\n",
        ));
        let env = [("MY_RPC", "https://secret.example")];
        let r = resolve(&dir, &env, &[]).unwrap();
        assert_eq!(r.config.rpc_url, "https://secret.example");
        assert_eq!(r.secret_keys, vec!["rpc_url"]);
        let redacted = r.redacted();
        assert_eq!(redacted.rpc_url, "secret://<redacted>");
        assert!(!format!("{redacted:?}").contains("secret.example"));
    }

    #[test]
    fn secret_reference_in_env_layer() {
        let dir = setup(None);
        let r = resolve(
            &dir,
            &[("LAFIYA_TESTNET_RPC_URL", "secret://env/MY_RPC"), ("MY_RPC", "https://s.example")],
            &[],
        )
        .unwrap();
        assert_eq!(r.config.rpc_url, "https://s.example");
        assert_eq!(r.secret_keys, vec!["rpc_url"]);
    }

    #[test]
    fn unknown_secret_scheme_fails_closed() {
        let dir = setup(Some(
            "[testnet]\nrpc_url = \"secret://vault/foo\"\n",
        ));
        let err = resolve(&dir, &[], &[]).unwrap_err().to_string();
        assert!(err.contains("vault"), "{err}");
    }

    #[test]
    fn malformed_secret_reference_fails_closed() {
        let dir = setup(Some("[testnet]\nrpc_url = \"secret://env\"\n"));
        let err = resolve(&dir, &[], &[]).unwrap_err().to_string();
        assert!(err.contains("secret://env"), "{err}");
    }

    #[test]
    fn plain_values_pass_through_resolver() {
        let resolver = SecretResolver::with_defaults();
        assert_eq!(resolver.resolve("https://x.example").unwrap(), "https://x.example");
    }

    #[test]
    fn profile_round_trips() {
        let dir = setup(None);
        let path = dir.path().join("networks.toml");
        assert_eq!(load_profile(&path).unwrap(), None);

        let saved = save_profile(&path, "testnet").unwrap();
        assert_eq!(saved.name, "testnet");
        assert!(matches!(saved.source, Source::File(_)));

        let loaded = load_profile(&path).unwrap().unwrap();
        assert_eq!(loaded.name, "testnet");
        assert_eq!(loaded, saved);
    }

    #[test]
    fn profile_must_name_a_known_network() {
        let dir = setup(None);
        let path = dir.path().join("networks.toml");
        let err = save_profile(&path, "miannet").unwrap_err().to_string();
        assert!(err.contains("miannet"), "{err}");
        assert!(!profile_path(&path).exists());
    }

    #[test]
    fn empty_profile_is_rejected() {
        let dir = setup(None);
        let path = dir.path().join("networks.toml");
        let err = save_profile(&path, "  ").unwrap_err().to_string();
        assert!(err.contains("profile"), "{err}");
    }

    #[test]
    fn profile_unknown_key_is_rejected() {
        let dir = setup(None);
        fs::write(dir.path().join("profile.toml"), "profil = \"testnet\"\n").unwrap();
        let err = load_profile(&dir.path().join("networks.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown key `profil`"), "{err}");
        assert!(err.contains("did you mean `profile`?"), "{err}");
    }

    #[test]
    fn profile_resolution_precedence() {
        let dir = setup(None);
        let path = dir.path().join("networks.toml");
        save_profile(&path, "testnet").unwrap();

        let env = |k: &str| (k == PROFILE_ENV_VAR).then(|| "local".to_string());

        let from_file = resolve_profile(&path, None, |_| None).unwrap().unwrap();
        assert_eq!(from_file.name, "testnet");
        assert!(matches!(from_file.source, Source::File(_)));

        let from_env = resolve_profile(&path, None, env).unwrap().unwrap();
        assert_eq!(from_env.name, "local");
        assert_eq!(from_env.source, Source::Env(PROFILE_ENV_VAR.into()));

        let from_flag = resolve_profile(&path, Some("mainnet"), env).unwrap().unwrap();
        assert_eq!(from_flag.name, "mainnet");
        assert_eq!(from_flag.source, Source::Flag);
    }

    #[test]
    fn no_profile_selected() {
        let dir = setup(None);
        let path = dir.path().join("networks.toml");
        assert_eq!(resolve_profile(&path, None, |_| None).unwrap(), None);
    }

    #[test]
    fn mainnet_detection() {
        for name in ["mainnet", "Mainnet", "MAINNET", "main-net", "main_net", "pubnet"] {
            assert!(is_mainnet(name), "{name}");
        }
        for name in ["testnet", "local", "futurenet", "main"] {
            assert!(!is_mainnet(name), "{name}");
        }
        let profile = ActiveProfile {
            name: "mainnet".to_string(),
            source: Source::Flag,
        };
        assert!(profile.is_mainnet());
    }
}
