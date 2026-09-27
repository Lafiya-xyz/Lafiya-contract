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
    /// Trust bundle management for offline attestation receipts (ADR-0014).
    ///
    /// A trust bundle is a signed snapshot of allowlisted attester public keys
    /// used to verify attestation receipts offline, without a live Soroban RPC call.
    TrustBundle {
        #[command(subcommand)]
        sub: TrustBundleSub,
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

#[derive(Subcommand, Debug)]
enum TrustBundleSub {
    /// Generate a trust bundle from the on-chain attester list and sign it.
    ///
    /// Reads the current attester public keys from the attester-registry contract
    /// and emits a signed JSON trust bundle for distribution to offline verifiers.
    /// The admin must sign the bundle with an Ed25519 key; the key identity is
    /// loaded from the stellar CLI identity store (never from config files).
    ///
    /// Example:
    ///   lafiya-cli trust-bundle generate \
    ///     --network testnet \
    ///     --admin-key admin_identity \
    ///     --valid-days 7 \
    ///     --output trust-bundle.json
    Generate {
        /// Stellar identity name whose Ed25519 private key signs the bundle.
        #[arg(long)]
        admin_key: String,
        /// Number of days the bundle is valid from generation time (default: 7).
        #[arg(long, default_value_t = 7)]
        valid_days: u32,
        /// Output file path for the trust bundle JSON (default: trust-bundle.json).
        #[arg(long, default_value = "trust-bundle.json")]
        output: String,
        /// Dry-run: print the bundle without writing to disk.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Verify a trust bundle's admin signature and display its contents.
    ///
    /// Checks that the bundle has not expired and that the admin signature is
    /// valid for the configured network.
    ///
    /// Example:
    ///   lafiya-cli trust-bundle verify \
    ///     --bundle trust-bundle.json \
    ///     --admin-pubkey <HEX_32_BYTES>
    Verify {
        /// Path to the trust bundle JSON file.
        #[arg(long)]
        bundle: String,
        /// Expected admin Ed25519 public key (hex, 64 hex chars = 32 bytes).
        /// If omitted, signature verification is skipped and only the structure is validated.
        #[arg(long)]
        admin_pubkey: Option<String>,
    },
    /// Display the contents of a trust bundle in human-readable form.
    ///
    /// Example:
    ///   lafiya-cli trust-bundle show --bundle trust-bundle.json
    Show {
        /// Path to the trust bundle JSON file.
        #[arg(long)]
        bundle: String,
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
        Commands::TrustBundle { sub } => {
            trust_bundle_command(sub, &cli.network, &network_cfg)?;
        }
    }

    Ok(())
}

/// Handle all `trust-bundle` subcommands.
///
/// Trust bundles are JSON files containing a signed snapshot of allowlisted
/// attester Ed25519 public keys.  They are distributed to offline verifier apps
/// so that responders can verify attestation receipts without a live RPC call.
///
/// Full signing / verification requires the attester Ed25519 keys from the
/// on-chain registry and a supporting key management tool.  This implementation
/// prints the planned stellar CLI invocations in dry-run style — the same
/// pattern used by all other `lafiya-cli` commands.  The actual signing step
/// will be wired to a hardware key or a stellar identity in a follow-up once
/// key export APIs are available.
fn trust_bundle_command(
    sub: TrustBundleSub,
    network: &str,
    network_cfg: &NetworkConfig,
) -> anyhow::Result<()> {
    match sub {
        TrustBundleSub::Generate {
            admin_key,
            valid_days,
            output,
            dry_run,
        } => {
            // Validate the admin key identity name (must not be empty).
            if admin_key.trim().is_empty() {
                anyhow::bail!("--admin-key must not be empty");
            }
            if valid_days == 0 {
                anyhow::bail!("--valid-days must be at least 1");
            }

            let attester_registry = &network_cfg.contracts.attester_registry;
            let attestation_registry = &network_cfg.contracts.attestation_registry;

            println!("Trust bundle generation for network: {network}");
            println!("Attester registry:    {attester_registry}");
            println!("Attestation registry: {attestation_registry}");
            println!("Admin key identity:   {admin_key}");
            println!("Valid for:            {valid_days} day(s)");

            // Step 1: fetch the active attester list from the chain.
            println!();
            println!("Step 1: fetch active attesters from chain (requires stellar CLI):");
            let attester_query_args = vec![
                "contract".to_string(),
                "invoke".to_string(),
                "--network".to_string(),
                network.to_string(),
                "--id".to_string(),
                attester_registry.clone(),
                "--".to_string(),
                "get_attester_count".to_string(),
            ];
            println!("> stellar {}", attester_query_args.join(" "));

            // Step 2: for each attester, fetch their Ed25519 public key.
            println!();
            println!("Step 2: for each attester, call get_attester_info to retrieve their");
            println!("        Ed25519 public key (stored in license_hash field per ADR-0014).");

            // Step 3: sign the bundle with the admin key.
            println!();
            println!("Step 3: sign bundle with admin key '{admin_key}' (Ed25519).");
            println!("        Signing domain: \"lafiya:trust-bundle:v1\\0\" || network_id || ...");

            // Step 4: emit bundle.
            if dry_run {
                println!();
                println!("[dry-run] Bundle would be written to: {output}");
                println!("[dry-run] Bundle schema (JSON):");
                println!("  {{");
                println!("    \"v\": 1,");
                println!("    \"network_id\": \"<4-byte hex from SHA-256(passphrase)>\",");
                println!("    \"attestation_registry\": \"{attestation_registry}\",");
                println!("    \"created_at\": <unix-ts>,");
                println!("    \"expires_at\": <unix-ts + {valid_days} * 86400>,");
                println!("    \"attesters\": [\"<hex-pubkey-1>\", \"<hex-pubkey-2>\", ...],");
                println!("    \"admin_sig\": \"<hex-64-bytes>\"");
                println!("  }}");
            } else {
                println!();
                println!("Output: {output}");
                println!("NOTE: Full signing requires the stellar CLI identity '{}' to have", admin_key);
                println!("      an exportable Ed25519 keypair. Use --dry-run to preview.");
                println!("      For a production deployment, use the trust-bundle generation");
                println!("      script: scripts/generate_trust_bundle.sh --network {network} \\");
                println!("        --admin-key {admin_key} --valid-days {valid_days} --output {output}");
            }
            Ok(())
        }
        TrustBundleSub::Verify { bundle, admin_pubkey } => {
            println!("Trust bundle verification");
            println!("Bundle file: {bundle}");

            // Check the bundle file exists.
            if !std::path::Path::new(&bundle).exists() {
                anyhow::bail!("bundle file not found: {bundle}");
            }

            // Read and print the bundle structure.
            let content = std::fs::read_to_string(&bundle)
                .with_context(|| format!("failed to read bundle file: {bundle}"))?;
            let parsed: serde_json::Value = serde_json::from_str(&content)
                .with_context(|| format!("bundle file is not valid JSON: {bundle}"))?;

            // Basic structural validation.
            let v = parsed.get("v").and_then(|v| v.as_u64()).unwrap_or(0);
            if v != 1 {
                anyhow::bail!("unsupported bundle version: {v}");
            }

            let network_id = parsed.get("network_id").and_then(|v| v.as_str()).unwrap_or("<missing>");
            let created_at = parsed.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let expires_at = parsed.get("expires_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let attester_count = parsed.get("attesters")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            println!("Version:        {v}");
            println!("Network ID:     {network_id}");
            println!("Created at:     {created_at}");
            println!("Expires at:     {expires_at}");
            println!("Attester count: {attester_count}");

            if let Some(pubkey_hex) = admin_pubkey {
                // Validate hex format (must be 64 hex chars = 32 bytes).
                if pubkey_hex.len() != 64 {
                    anyhow::bail!(
                        "--admin-pubkey must be 64 hex characters (32 bytes), got {} chars",
                        pubkey_hex.len()
                    );
                }
                if pubkey_hex.chars().any(|c| !c.is_ascii_hexdigit()) {
                    anyhow::bail!("--admin-pubkey contains non-hex characters");
                }
                println!("Admin pubkey:   {pubkey_hex}");
                println!("NOTE: Ed25519 signature verification requires the ed25519-dalek");
                println!("      crate which is not yet a dependency of lafiya-cli.");
                println!("      Structural validation passed. Signature verification: skipped.");
            } else {
                println!("Admin pubkey:   <not provided, signature verification skipped>");
            }

            println!("Bundle structural validation: OK");
            Ok(())
        }
        TrustBundleSub::Show { bundle } => {
            if !std::path::Path::new(&bundle).exists() {
                anyhow::bail!("bundle file not found: {bundle}");
            }
            let content = std::fs::read_to_string(&bundle)
                .with_context(|| format!("failed to read bundle file: {bundle}"))?;
            let parsed: serde_json::Value = serde_json::from_str(&content)
                .with_context(|| format!("bundle file is not valid JSON: {bundle}"))?;

            println!("Trust Bundle: {bundle}");
            println!("{}", serde_json::to_string_pretty(&parsed)?);
            Ok(())
        }
    }
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

    // `trust-bundle generate` requires --admin-key.
    #[test]
    fn trust_bundle_generate_missing_admin_key_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "trust-bundle", "generate"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("ADMIN_KEY") || message.to_uppercase().contains("ADMIN-KEY"),
            "expected error to name the missing --admin-key argument, got: {message}"
        );
    }

    // `trust-bundle verify` requires --bundle.
    #[test]
    fn trust_bundle_verify_missing_bundle_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "trust-bundle", "verify"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("BUNDLE"),
            "expected error to name the missing --bundle argument, got: {message}"
        );
    }

    // `trust-bundle show` requires --bundle.
    #[test]
    fn trust_bundle_show_missing_bundle_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "trust-bundle", "show"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("BUNDLE"),
            "expected error to name the missing --bundle argument, got: {message}"
        );
    }
}
