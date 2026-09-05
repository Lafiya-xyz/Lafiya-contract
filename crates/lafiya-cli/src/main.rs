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
        /// Stellar address (G...) to allowlist as an attester
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Remove attester
    Remove {
        /// Stellar address (G...) to remove from the allowlist
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

    // `attester add` requires a positional `address`. Clap's derive-generated
    // error for a missing required argument must still name it, so a
    // contributor testing the CLI by hand isn't left guessing which value
    // they forgot.
    #[test]
    fn attester_add_missing_address_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "attester", "add"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("ADDRESS"),
            "expected error to name the missing `address` argument, got: {message}"
        );
    }

    // `attestation get` requires a positional `record_hash`.
    #[test]
    fn attestation_get_missing_record_hash_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "attestation", "get"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("RECORD_HASH"),
            "expected error to name the missing `record_hash` argument, got: {message}"
        );
    }
}
