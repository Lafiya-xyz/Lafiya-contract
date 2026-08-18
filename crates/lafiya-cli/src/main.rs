//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.
//!
//! Every operator supplied value (network name, address, contract id, record
//! hash, admin/source account) is validated locally before the stellar CLI is
//! invoked, so malformed input fails fast with an actionable message.

use anyhow::Context;
use clap::{Parser, Subcommand};
use lafiya_config::{
    get_network, load_networks, validate_account_address, validate_address, validate_network_name,
    validate_record_hash, validate_source_account, ContractKind, DeploymentState, NetworkConfig,
};
use std::path::PathBuf;

/// Env var holding the stellar CLI identity used as transaction source.
const ENV_SOURCE: &str = "STELLAR_ACCOUNT";
/// Env var holding the contract admin address.
const ENV_ADMIN: &str = "ADMIN_ADDRESS";

#[derive(Parser, Debug)]
#[command(
    name = "lafiya-cli",
    about = "Lafiya Admin CLI - uses config/networks.toml"
)]
struct Cli {
    /// Network name as defined in config/networks.toml (e.g. testnet, futurenet, mainnet, local)
    #[arg(long, default_value = "testnet", global = true)]
    network: String,

    /// Path to networks.toml (auto-discovers by default)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show / list network config
    Config {
        #[command(subcommand)]
        sub: ConfigSub,
    },
    /// Attester registry operations
    Attester {
        #[command(subcommand)]
        sub: AttesterSub,
    },
    /// Attestation registry operations
    Attestation {
        #[command(subcommand)]
        sub: AttestationSub,
    },
    /// Deploy contracts (wrapper around scripts/deploy.sh logic, but uses same config)
    Deploy {
        /// Build only, don't deploy
        #[arg(long, default_value_t = false)]
        build_only: bool,
        /// Dry run
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Stellar identity or G... address used as transaction source (or STELLAR_ACCOUNT)
        #[arg(long)]
        source: Option<String>,
        /// Admin address (G...) for contract initialization (or ADMIN_ADDRESS)
        #[arg(long)]
        admin: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigSub {
    /// Show resolved config for selected network
    Show,
    /// List all available networks in config
    List,
    /// Print shell export lines for current network (for use with eval or sourcing)
    Env,
}

#[derive(Subcommand, Debug)]
enum AttesterSub {
    /// Check if an address is allowlisted
    Is {
        /// Stellar address (G...)
        address: String,
    },
    /// Add attester (requires admin - will invoke stellar CLI)
    Add {
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Remove attester
    Remove {
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum AttestationSub {
    /// Get attestation for a record hash (hex encoded 32-byte hash)
    Get {
        /// Hex string of 32-byte record hash (64 chars)
        record_hash: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Validate the network name before it is used as a config key.
    validate_network_name(&cli.network)
        .map_err(|e| anyhow::anyhow!("invalid --network value: {e}"))?;

    let config_path_opt = cli.config.as_deref();
    let networks = load_networks(config_path_opt)?;

    // For config list, we don't need to resolve specific network
    if let Commands::Config {
        sub: ConfigSub::List,
    } = &cli.command
    {
        println!(
            "Available networks (from {:?}):",
            lafiya_config::default_config_path()
        );
        for name in networks.keys() {
            println!("  - {}", name);
        }
        if let Some(p) = &cli.config {
            println!("Config path (explicit): {:?}", p);
        } else {
            let default = lafiya_config::default_config_path();
            println!("Config path (auto): {:?}", default);
        }
        return Ok(());
    }

    let network_cfg = get_network(&networks, &cli.network).map_err(|e| anyhow::anyhow!(e))?;

    // `config show` reports config problems instead of refusing to print, so an
    // operator can see exactly which value needs fixing. Every other command
    // requires a valid profile before touching the network.
    let is_config_show = matches!(
        cli.command,
        Commands::Config {
            sub: ConfigSub::Show
        }
    );
    if let Err(e) = network_cfg.validate(&cli.network) {
        if is_config_show {
            eprintln!("WARNING: {e}");
        } else {
            return Err(anyhow::anyhow!(e));
        }
    }

    match cli.command {
        Commands::Config { sub } => {
            match sub {
                ConfigSub::Show => {
                    let (path, _) = lafiya_config::load_network_config::<PathBuf>(
                        &cli.network,
                        cli.config.clone(),
                    )?;
                    println!("Network: {}", cli.network);
                    println!("Config: {:?}", path);
                    println!("RPC URL: {}", network_cfg.rpc_url);
                    println!("Passphrase: {}", network_cfg.network_passphrase);
                    println!(
                        "Attester registry: {}",
                        if network_cfg.contracts.attester_registry.is_empty() {
                            "<not deployed>".to_string()
                        } else {
                            network_cfg.contracts.attester_registry.clone()
                        }
                    );
                    println!(
                        "Attestation registry: {}",
                        if network_cfg.contracts.attestation_registry.is_empty() {
                            "<not deployed>".to_string()
                        } else {
                            network_cfg.contracts.attestation_registry.clone()
                        }
                    );
                    println!("Deployed: {}", network_cfg.is_deployed());
                    println!("Deployment status: {}", deployment_summary(&network_cfg));
                    println!("\nSecrets: NEVER stored in networks.toml. Use stellar identities or env vars.");
                }
                ConfigSub::List => {} // handled above
                ConfigSub::Env => {
                    println!(
                        "# Source this with: eval $(lafiya-cli --network {} config env)",
                        cli.network
                    );
                    println!("export LAFIYA_NETWORK={}", cli.network);
                    println!("export LAFIYA_RPC_URL={}", network_cfg.rpc_url);
                    println!(
                        "export LAFIYA_NETWORK_PASSPHRASE={:?}",
                        network_cfg.network_passphrase
                    );
                    println!(
                        "export LAFIYA_ATTESTER_REGISTRY_ID={}",
                        network_cfg.contracts.attester_registry
                    );
                    println!(
                        "export LAFIYA_ATTESTATION_REGISTRY_ID={}",
                        network_cfg.contracts.attestation_registry
                    );
                }
            }
        }
        Commands::Attester { sub } => match sub {
            AttesterSub::Is { address } => {
                let contract_id = network_cfg
                    .require_contract_id(&cli.network, ContractKind::AttesterRegistry)
                    .map_err(|e| anyhow::anyhow!(e))?;
                validate_address("attester address", &address)
                    .context("invalid attester address")?;

                println!("Checking is_attester for {} on {}", address, contract_id);
                println!("RPC: {}", network_cfg.rpc_url);
                let args = invoke_args(
                    &network_cfg,
                    contract_id,
                    None,
                    "is_attester",
                    &["--attester", &address],
                );
                println!("> stellar {}", args.join(" "));
                // Read-only query: report a missing/failing CLI without aborting hard.
                if which::which("stellar").is_ok() {
                    if let Err(e) = std::process::Command::new("stellar").args(args).status() {
                        eprintln!("Failed to run stellar CLI: {e}. Install with: cargo install --locked stellar-cli");
                    }
                } else {
                    eprintln!("stellar CLI not found - showing command only. Install with: cargo install --locked stellar-cli");
                }
            }
            AttesterSub::Add { address, source } => {
                let contract_id = network_cfg
                    .require_contract_id(&cli.network, ContractKind::AttesterRegistry)
                    .map_err(|e| anyhow::anyhow!(e))?;
                validate_address("attester address", &address)
                    .context("invalid attester address")?;
                let source = validated_source(source)?;

                let args = invoke_args(
                    &network_cfg,
                    contract_id,
                    source.as_deref(),
                    "add_attester",
                    &["--attester", &address],
                );
                run_stellar(args)?;
            }
            AttesterSub::Remove { address, source } => {
                let contract_id = network_cfg
                    .require_contract_id(&cli.network, ContractKind::AttesterRegistry)
                    .map_err(|e| anyhow::anyhow!(e))?;
                validate_address("attester address", &address)
                    .context("invalid attester address")?;
                let source = validated_source(source)?;

                let args = invoke_args(
                    &network_cfg,
                    contract_id,
                    source.as_deref(),
                    "remove_attester",
                    &["--attester", &address],
                );
                run_stellar(args)?;
            }
        },
        Commands::Attestation { sub } => match sub {
            AttestationSub::Get { record_hash } => {
                let contract_id = network_cfg
                    .require_contract_id(&cli.network, ContractKind::AttestationRegistry)
                    .map_err(|e| anyhow::anyhow!(e))?;
                validate_record_hash("record_hash", &record_hash)
                    .context("invalid record hash (expected a hex encoded 32-byte hash)")?;

                let args = invoke_args(
                    &network_cfg,
                    contract_id,
                    None,
                    "get_attestation",
                    &["--record_hash", &record_hash],
                );
                println!("> stellar {}", args.join(" "));
                if which::which("stellar").is_ok() {
                    let status = std::process::Command::new("stellar").args(args).status()?;
                    if !status.success() {
                        anyhow::bail!("stellar CLI failed");
                    }
                } else {
                    eprintln!(
                        "stellar CLI not found - install with cargo install --locked stellar-cli"
                    );
                }
            }
        },
        Commands::Deploy {
            build_only,
            dry_run,
            source,
            admin,
        } => {
            let identity = DeployIdentity::resolve(
                admin,
                source,
                std::env::var(ENV_ADMIN).ok(),
                std::env::var(ENV_SOURCE).ok(),
                DeployMode::new(build_only, dry_run),
            )?;

            println!("Deploy flow for network: {}", cli.network);
            println!("RPC: {}", network_cfg.rpc_url);
            println!("Passphrase: {}", network_cfg.network_passphrase);
            println!("Current deployment: {}", deployment_summary(&network_cfg));
            println!("Source: {}", identity.source.as_deref().unwrap_or("<none>"));
            println!("Admin: {}", identity.admin.as_deref().unwrap_or("<none>"));
            println!("This command is a wrapper- for full deploy use:");
            println!("  ./scripts/deploy.sh --network {}", cli.network);
            if build_only {
                println!("Building WASM...");
                let status = std::process::Command::new("cargo")
                    .args([
                        "build",
                        "--workspace",
                        "--release",
                        "--target",
                        "wasm32v1-none",
                    ])
                    .status()?;
                if !status.success() {
                    anyhow::bail!("build failed");
                }
            }
            if dry_run {
                println!(
                    "[dry-run] Would deploy attester-registry and attestation-registry to {}",
                    cli.network
                );
            }
        }
    }

    Ok(())
}

/// What a `deploy` invocation is actually allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeployMode {
    /// Builds WASM only, never touches a network.
    BuildOnly,
    /// Prints the plan, never touches a network.
    DryRun,
    /// Would submit transactions, so identity configuration is mandatory.
    Live,
}

impl DeployMode {
    fn new(build_only: bool, dry_run: bool) -> Self {
        // build-only and dry-run are both offline; neither needs credentials.
        if build_only {
            DeployMode::BuildOnly
        } else if dry_run {
            DeployMode::DryRun
        } else {
            DeployMode::Live
        }
    }

    fn requires_identity(self) -> bool {
        matches!(self, DeployMode::Live)
    }
}

/// Admin / source values resolved from flags then environment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeployIdentity {
    admin: Option<String>,
    source: Option<String>,
}

impl DeployIdentity {
    /// Resolve and validate deployment identity.
    ///
    /// Flags win over environment. Outside dry-run/build-only a live deployment
    /// refuses to start without both an admin address and a transaction source,
    /// because a half-configured deployment leaves contracts uninitialized.
    fn resolve(
        admin_flag: Option<String>,
        source_flag: Option<String>,
        admin_env: Option<String>,
        source_env: Option<String>,
        mode: DeployMode,
    ) -> anyhow::Result<Self> {
        let admin = first_non_empty(admin_flag, admin_env);
        let source = first_non_empty(source_flag, source_env);

        if let Some(admin) = &admin {
            validate_account_address("admin", admin)
                .context("invalid --admin value (expected a G... account address)")?;
        }
        if let Some(source) = &source {
            validate_source_account(source).context(
                "invalid --source value (expected a stellar identity name or G... address)",
            )?;
        }

        if mode.requires_identity() {
            if source.is_none() {
                anyhow::bail!(
                    "deployment requires a transaction source: pass --source <identity> or set {ENV_SOURCE} (use --dry-run to preview without credentials)"
                );
            }
            if admin.is_none() {
                anyhow::bail!(
                    "deployment requires an admin address: pass --admin <G...> or set {ENV_ADMIN} (use --dry-run to preview without credentials)"
                );
            }
        }

        Ok(Self { admin, source })
    }
}

fn first_non_empty(primary: Option<String>, fallback: Option<String>) -> Option<String> {
    primary
        .into_iter()
        .chain(fallback)
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// Validate an optional `--source` before it reaches the stellar CLI.
fn validated_source(source: Option<String>) -> anyhow::Result<Option<String>> {
    match first_non_empty(source, None) {
        Some(src) => {
            validate_source_account(&src).context(
                "invalid --source value (expected a stellar identity name or G... address)",
            )?;
            Ok(Some(src))
        }
        None => Ok(None),
    }
}

/// Build a `stellar contract invoke` argument list for the given network profile.
fn invoke_args(
    cfg: &NetworkConfig,
    contract_id: &str,
    source: Option<&str>,
    function: &str,
    function_args: &[&str],
) -> Vec<String> {
    let mut args = vec![
        "contract".to_string(),
        "invoke".to_string(),
        "--id".to_string(),
        contract_id.to_string(),
        "--rpc-url".to_string(),
        cfg.rpc_url.clone(),
        "--network-passphrase".to_string(),
        cfg.network_passphrase.clone(),
    ];
    if let Some(src) = source {
        args.push("--source".to_string());
        args.push(src.to_string());
    }
    args.push("--".to_string());
    args.push(function.to_string());
    args.extend(function_args.iter().map(|a| a.to_string()));
    args
}

/// Print and run a stellar CLI invocation, failing loudly if it is unavailable.
fn run_stellar(args: Vec<String>) -> anyhow::Result<()> {
    println!("> stellar {}", args.join(" "));
    if which::which("stellar").is_err() {
        anyhow::bail!("stellar CLI not found");
    }
    let status = std::process::Command::new("stellar").args(args).status()?;
    if !status.success() {
        anyhow::bail!("stellar CLI failed");
    }
    Ok(())
}

/// Human readable deployment state, including partially deployed profiles.
fn deployment_summary(cfg: &NetworkConfig) -> String {
    match cfg.deployment_state() {
        DeploymentState::Deployed => "fully deployed".to_string(),
        DeploymentState::NotDeployed => "not deployed".to_string(),
        DeploymentState::Partial { missing } => {
            let missing = missing
                .iter()
                .map(|k| k.key())
                .collect::<Vec<_>>()
                .join(", ");
            format!("PARTIALLY DEPLOYED - missing contract id(s): {missing}")
        }
    }
}

// Tiny which implementation to avoid extra dep if not available, but we add which crate feature? We'll implement simple check
mod which {
    use std::path::Path;

    pub fn which(bin: &str) -> Result<std::path::PathBuf, ()> {
        // Simple check using PATH env
        if let Some(paths) = std::env::var_os("PATH") {
            for p in std::env::split_paths(&paths) {
                let full = p.join(bin);
                if full.exists() {
                    return Ok(full);
                }
                // Windows also .exe etc, but we target unix for stellar
                #[cfg(windows)]
                {
                    let full_exe = p.join(format!("{}.exe", bin));
                    if full_exe.exists() {
                        return Ok(full_exe);
                    }
                }
                // Also check without extension but with executable bit
                if Path::new(&full).exists() {
                    return Ok(full);
                }
            }
        }
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lafiya_config::ContractIds;

    const ADMIN: &str = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
    const ATTESTER_ID: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const ATTESTATION_ID: &str = "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX";

    fn cfg(attester: &str, attestation: &str) -> NetworkConfig {
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
    fn invoke_args_include_source_only_when_provided() {
        let cfg = cfg(ATTESTER_ID, ATTESTATION_ID);
        let without = invoke_args(
            &cfg,
            ATTESTER_ID,
            None,
            "is_attester",
            &["--attester", ADMIN],
        );
        assert!(!without.contains(&"--source".to_string()));
        assert_eq!(
            without,
            vec![
                "contract",
                "invoke",
                "--id",
                ATTESTER_ID,
                "--rpc-url",
                "https://soroban-testnet.stellar.org",
                "--network-passphrase",
                "Test SDF Network ; September 2015",
                "--",
                "is_attester",
                "--attester",
                ADMIN,
            ]
        );

        let with = invoke_args(
            &cfg,
            ATTESTER_ID,
            Some("deployer"),
            "add_attester",
            &["--attester", ADMIN],
        );
        let source_pos = with.iter().position(|a| a == "--source").unwrap();
        assert_eq!(with[source_pos + 1], "deployer");
        // The source flag belongs to stellar, before the `--` separator.
        assert!(source_pos < with.iter().position(|a| a == "--").unwrap());
    }

    #[test]
    fn deployment_summary_reports_partial_profiles() {
        assert_eq!(
            deployment_summary(&cfg(ATTESTER_ID, ATTESTATION_ID)),
            "fully deployed"
        );
        assert_eq!(deployment_summary(&cfg("", "")), "not deployed");
        let partial = deployment_summary(&cfg(ATTESTER_ID, ""));
        assert!(partial.contains("PARTIALLY DEPLOYED"), "{partial}");
        assert!(partial.contains("attestation_registry"), "{partial}");
    }

    #[test]
    fn live_deploy_requires_admin_and_source() {
        let err = DeployIdentity::resolve(None, None, None, None, DeployMode::Live)
            .unwrap_err()
            .to_string();
        assert!(err.contains("transaction source"), "{err}");

        let err =
            DeployIdentity::resolve(None, Some("deployer".into()), None, None, DeployMode::Live)
                .unwrap_err()
                .to_string();
        assert!(err.contains("admin address"), "{err}");

        let ok = DeployIdentity::resolve(
            Some(ADMIN.into()),
            Some("deployer".into()),
            None,
            None,
            DeployMode::Live,
        )
        .unwrap();
        assert_eq!(ok.admin.as_deref(), Some(ADMIN));
        assert_eq!(ok.source.as_deref(), Some("deployer"));
    }

    #[test]
    fn dry_run_and_build_only_do_not_require_identity() {
        for mode in [DeployMode::DryRun, DeployMode::BuildOnly] {
            let identity = DeployIdentity::resolve(None, None, None, None, mode).unwrap();
            assert_eq!(
                identity,
                DeployIdentity {
                    admin: None,
                    source: None
                }
            );
        }
    }

    #[test]
    fn deploy_identity_falls_back_to_environment_and_ignores_blanks() {
        let identity = DeployIdentity::resolve(
            None,
            Some("   ".into()),
            Some(ADMIN.into()),
            Some("env-deployer".into()),
            DeployMode::Live,
        )
        .unwrap();
        assert_eq!(identity.admin.as_deref(), Some(ADMIN));
        assert_eq!(identity.source.as_deref(), Some("env-deployer"));
    }

    #[test]
    fn deploy_identity_rejects_malformed_values_even_in_dry_run() {
        let err = DeployIdentity::resolve(
            Some(ATTESTER_ID.into()),
            None,
            None,
            None,
            DeployMode::DryRun,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--admin"), "{err}");

        let err = DeployIdentity::resolve(
            Some(ADMIN.into()),
            Some("deployer; curl evil.example".into()),
            None,
            None,
            DeployMode::DryRun,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--source"), "{err}");
    }

    #[test]
    fn validated_source_accepts_none_and_rejects_malformed() {
        assert_eq!(validated_source(None).unwrap(), None);
        assert_eq!(validated_source(Some("  ".into())).unwrap(), None);
        assert_eq!(
            validated_source(Some("deployer".into()))
                .unwrap()
                .as_deref(),
            Some("deployer")
        );
        assert!(validated_source(Some("bad source".into())).is_err());
    }

    #[test]
    fn deploy_mode_classification() {
        assert_eq!(DeployMode::new(true, false), DeployMode::BuildOnly);
        assert_eq!(DeployMode::new(false, true), DeployMode::DryRun);
        assert_eq!(DeployMode::new(false, false), DeployMode::Live);
        // build-only wins: nothing is submitted, so no credentials are needed.
        assert_eq!(DeployMode::new(true, true), DeployMode::BuildOnly);
    }
}
