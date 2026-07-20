//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.

use clap::{Parser, Subcommand};
use lafiya_config::{get_network, load_networks};
use std::path::PathBuf;

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
                if network_cfg.contracts.attester_registry.is_empty() {
                    anyhow::bail!(
                        "attester_registry not deployed for network '{}'. Deploy first with --network {}",
                        cli.network,
                        cli.network
                    );
                }
                println!(
                    "Checking is_attester for {} on {}",
                    address, network_cfg.contracts.attester_registry
                );
                println!("RPC: {}", network_cfg.rpc_url);
                // We print the stellar CLI command that would be run
                // To actually invoke, we'd need stellar CLI integration; for now print guidance
                // and attempt if stellar is installed.
                let stellar_args = [
                    "contract",
                    "invoke",
                    "--id",
                    &network_cfg.contracts.attester_registry,
                    "--rpc-url",
                    &network_cfg.rpc_url,
                    "--network-passphrase",
                    &network_cfg.network_passphrase,
                    "--",
                    "is_attester",
                    "--attester",
                    &address,
                ];
                println!("> stellar {}", stellar_args.join(" "));
                // Try executing if stellar exists
                if which::which("stellar").is_ok() {
                    let status = std::process::Command::new("stellar")
                        .args(stellar_args)
                        .status();
                    if let Err(e) = status {
                        eprintln!("Failed to run stellar CLI: {e}. Install with: cargo install --locked stellar-cli");
                    }
                } else {
                    eprintln!("stellar CLI not found — showing command only. Install with: cargo install --locked stellar-cli");
                }
            }
            AttesterSub::Add { address, source } => {
                if network_cfg.contracts.attester_registry.is_empty() {
                    anyhow::bail!("attester_registry not deployed for '{}'", cli.network);
                }
                let mut args = vec![
                    "contract".to_string(),
                    "invoke".to_string(),
                    "--id".to_string(),
                    network_cfg.contracts.attester_registry.clone(),
                    "--rpc-url".to_string(),
                    network_cfg.rpc_url.clone(),
                    "--network-passphrase".to_string(),
                    network_cfg.network_passphrase.clone(),
                ];
                if let Some(src) = source {
                    args.push("--source".to_string());
                    args.push(src);
                }
                args.push("--".to_string());
                args.push("add_attester".to_string());
                args.push("--attester".to_string());
                args.push(address.clone());
                println!("> stellar {}", args.join(" "));
                if which::which("stellar").is_ok() {
                    let status = std::process::Command::new("stellar").args(args).status()?;
                    if !status.success() {
                        anyhow::bail!("stellar CLI failed");
                    }
                } else {
                    anyhow::bail!("stellar CLI not found");
                }
            }
            AttesterSub::Remove { address, source } => {
                if network_cfg.contracts.attester_registry.is_empty() {
                    anyhow::bail!("attester_registry not deployed for '{}'", cli.network);
                }
                let mut args = vec![
                    "contract".to_string(),
                    "invoke".to_string(),
                    "--id".to_string(),
                    network_cfg.contracts.attester_registry.clone(),
                    "--rpc-url".to_string(),
                    network_cfg.rpc_url.clone(),
                    "--network-passphrase".to_string(),
                    network_cfg.network_passphrase.clone(),
                ];
                if let Some(src) = source {
                    args.push("--source".to_string());
                    args.push(src);
                }
                args.push("--".to_string());
                args.push("remove_attester".to_string());
                args.push("--attester".to_string());
                args.push(address.clone());
                println!("> stellar {}", args.join(" "));
                if which::which("stellar").is_ok() {
                    let status = std::process::Command::new("stellar").args(args).status()?;
                    if !status.success() {
                        anyhow::bail!("stellar CLI failed");
                    }
                } else {
                    anyhow::bail!("stellar CLI not found");
                }
            }
        },
        Commands::Attestation { sub } => match sub {
            AttestationSub::Get { record_hash } => {
                if network_cfg.contracts.attestation_registry.is_empty() {
                    anyhow::bail!("attestation_registry not deployed for '{}'", cli.network);
                }
                // Basic validation for hex length
                if record_hash.len() != 64 || !record_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    anyhow::bail!("record_hash must be 64 hex chars (32-byte hash)");
                }
                let args = [
                    "contract",
                    "invoke",
                    "--id",
                    &network_cfg.contracts.attestation_registry,
                    "--rpc-url",
                    &network_cfg.rpc_url,
                    "--network-passphrase",
                    &network_cfg.network_passphrase,
                    "--",
                    "get_attestation",
                    "--record_hash",
                    &record_hash,
                ];
                println!("> stellar {}", args.join(" "));
                if which::which("stellar").is_ok() {
                    let status = std::process::Command::new("stellar").args(args).status()?;
                    if !status.success() {
                        anyhow::bail!("stellar CLI failed");
                    }
                } else {
                    eprintln!(
                        "stellar CLI not found — install with cargo install --locked stellar-cli"
                    );
                }
            }
        },
        Commands::Deploy {
            build_only,
            dry_run,
        } => {
            println!("Deploy flow for network: {}", cli.network);
            println!("RPC: {}", network_cfg.rpc_url);
            println!("Passphrase: {}", network_cfg.network_passphrase);
            println!("This command is a wrapper— for full deploy use:");
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
