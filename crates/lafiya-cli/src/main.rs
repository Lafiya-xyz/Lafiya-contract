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

// ── Helper: build `stellar contract invoke` argv ─────────────────────────────

/// Build a `stellar contract invoke` argument list from the given parameters.
///
/// The returned `Vec<String>` is passed directly to `std::process::Command::args`,
/// so every argument is a separate element — values are never shell-concatenated
/// or interpolated, which prevents any injection through user-supplied fields.
///
/// # Arguments
/// * `cfg`         – network config (supplies `--rpc-url` and `--network-passphrase`)
/// * `contract_id` – the `C...` contract strkey for `--id`
/// * `source`      – optional stellar CLI identity or `G...` address for `--source`
/// * `fn_name`     – the contract function name for `--fn`
/// * `fn_args`     – the already-validated positional function arguments (e.g.
///                   `&["--attester", "G..."]`)
pub fn invoke_args(
    cfg: &NetworkConfig,
    contract_id: &str,
    source: Option<&str>,
    fn_name: &str,
    fn_args: &[&str],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
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
    args.push(fn_name.to_string());
    for a in fn_args {
        args.push(a.to_string());
    }
    args
}

/// Run `stellar <args>` as a subprocess, propagating a non-zero exit as an error.
///
/// If the `stellar` CLI binary is not on `PATH`, returns an error with install
/// instructions instead of panicking.
pub fn run_stellar(args: Vec<String>) -> anyhow::Result<()> {
    if which::which("stellar").is_err() {
        anyhow::bail!(
            "stellar CLI not found — install with: cargo install --locked stellar-cli"
        );
    }
    println!("> stellar {}", args.join(" "));
    let status = std::process::Command::new("stellar")
        .args(&args)
        .status()
        .context("failed to spawn stellar CLI")?;
    if !status.success() {
        anyhow::bail!("stellar CLI exited with {}", status);
    }
    Ok(())
}

/// Resolve the transaction source account.
///
/// Precedence (first non-empty wins):
/// 1. `--source` flag value
/// 2. `STELLAR_ACCOUNT` environment variable
///
/// The resolved value is validated with `validate_source_account` — it must be
/// a stellar CLI identity name or a `G...` strkey.  Returns `Ok(None)` only
/// when both sources are absent.
pub fn validated_source(flag: Option<String>) -> anyhow::Result<Option<String>> {
    let raw = flag.or_else(|| std::env::var(ENV_SOURCE).ok());
    match raw {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            validate_source_account(&s).context("invalid source account")?;
            Ok(Some(s))
        }
    }
}

/// Produce a human-readable one-liner summarising the deployment state of `cfg`.
pub fn deployment_summary(cfg: &NetworkConfig) -> String {
    match cfg.deployment_state() {
        DeploymentState::NotDeployed => "not yet deployed".to_string(),
        DeploymentState::Deployed => format!(
            "deployed (attester-registry: {}, attestation-registry: {})",
            cfg.contracts.attester_registry, cfg.contracts.attestation_registry
        ),
        DeploymentState::Partial { missing } => {
            let names: Vec<&str> = missing.iter().map(|k| k.key()).collect();
            format!("partially deployed — missing: {}", names.join(", "))
        }
    }
}

// ── Deploy identity and mode ──────────────────────────────────────────────────

/// The deploy mode selected by the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployMode {
    /// Only build the WASM artefacts; skip network submission.
    BuildOnly,
    /// Resolve network config and print what would happen, but do not submit.
    DryRun,
    /// Full deploy (build + upload + deploy).
    Deploy,
}

impl DeployMode {
    /// Construct a `DeployMode` from the `--build-only` and `--dry-run` flags.
    pub fn new(build_only: bool, dry_run: bool) -> Self {
        if build_only {
            DeployMode::BuildOnly
        } else if dry_run {
            DeployMode::DryRun
        } else {
            DeployMode::Deploy
        }
    }
}

/// Resolved admin identity and transaction source for the deploy command.
#[derive(Debug, Clone)]
pub struct DeployIdentity {
    /// The admin `G...` address that will be passed to `initialize`.
    pub admin: Option<String>,
    /// The stellar CLI identity or `G...` address used as the transaction source.
    pub source: Option<String>,
}

impl DeployIdentity {
    /// Resolve the deploy identity from CLI flags, env fallbacks, and the deploy
    /// mode.
    ///
    /// For `Deploy` mode, both `source` and `admin` must be present (either from
    /// flags or env vars).  For `BuildOnly` and `DryRun` they are optional.
    pub fn resolve(
        admin_flag: Option<String>,
        source_flag: Option<String>,
        admin_env: Option<String>,
        source_env: Option<String>,
        mode: DeployMode,
    ) -> anyhow::Result<Self> {
        // Env fallbacks
        let admin = admin_flag.or(admin_env).filter(|s| !s.is_empty());
        let source = source_flag.or(source_env).filter(|s| !s.is_empty());

        // Validate what we have
        if let Some(ref a) = admin {
            validate_account_address("admin", a).context("invalid admin address")?;
        }
        if let Some(ref s) = source {
            validate_source_account(s).context("invalid source account")?;
        }

        // For a real deploy both are required
        if mode == DeployMode::Deploy {
            if admin.is_none() {
                anyhow::bail!(
                    "admin address is required for deploy. Pass --admin G... or set {ENV_ADMIN}"
                );
            }
            if source.is_none() {
                anyhow::bail!(
                    "source account is required for deploy. Pass --source or set {ENV_SOURCE}"
                );
            }
        }

        Ok(DeployIdentity { admin, source })
    }
}

// ── CLI argument types ────────────────────────────────────────────────────────

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
    /// Admin operations (propose/accept admin transfer, get admin)
    Admin {
        #[command(subcommand)]
        sub: AdminSub,
    },
    /// Operations lifecycle (pause / unpause / is-paused) for a contract
    Ops {
        #[command(subcommand)]
        sub: OpsSub,
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

// ── attester subcommands ──────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum AttesterSub {
    /// Check if an address is allowlisted
    Is {
        /// Stellar address (G...)
        address: String,
    },
    /// Add attester (requires admin)
    Add {
        /// Stellar address (G...) to allowlist as an attester
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Add attester with optional metadata (requires admin)
    AddWithInfo {
        /// Stellar address (G...) to allowlist
        address: String,
        /// Hex-encoded 32-byte license hash (optional)
        #[arg(long)]
        license_hash: Option<String>,
        /// Region symbol, e.g. `NG-LA` (optional)
        #[arg(long)]
        region: Option<String>,
        #[arg(long)]
        source: Option<String>,
    },
    /// Update metadata for an already-allowlisted attester (requires admin)
    UpdateInfo {
        /// Stellar address (G...) of the attester to update
        address: String,
        /// Hex-encoded 32-byte license hash (optional)
        #[arg(long)]
        license_hash: Option<String>,
        /// Region symbol, e.g. `NG-LA` (optional)
        #[arg(long)]
        region: Option<String>,
        #[arg(long)]
        source: Option<String>,
    },
    /// Remove attester (requires admin)
    Remove {
        /// Stellar address (G...) to remove from the allowlist
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Suspend an allowlisted attester (requires admin)
    Suspend {
        /// Stellar address (G...) to suspend
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Reinstate a suspended attester (requires admin)
    Reinstate {
        /// Stellar address (G...) to reinstate
        address: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Get stored metadata for an attester
    GetInfo {
        /// Stellar address (G...)
        address: String,
    },
    /// Get attester metadata and suspension state
    GetStatus {
        /// Stellar address (G...)
        address: String,
    },
    /// Set the soft cap on the number of allowlisted attesters (requires admin)
    SetMaxAttesters {
        /// Maximum number of attesters
        max: u32,
        #[arg(long)]
        source: Option<String>,
    },
    /// Get the soft cap on the number of allowlisted attesters
    GetMaxAttesters,
    /// Get the current count of allowlisted attesters
    GetCount,
    /// Get the storage schema version
    GetSchemaVersion,
    /// Upgrade the contract wasm (requires admin)
    Upgrade {
        /// Hex-encoded 32-byte wasm hash of the already-uploaded wasm blob
        new_wasm_hash: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Run any pending storage-schema migration (requires admin)
    Migrate {
        #[arg(long)]
        source: Option<String>,
    },
}

// ── attestation subcommands ───────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum AttestationSub {
    /// Get latest attestation for a record hash (hex encoded 32-byte hash)
    Get {
        /// Hex string of 32-byte record hash (64 chars)
        record_hash: String,
    },
    /// Get full attestation history for a record hash
    GetHistory {
        /// Hex string of 32-byte record hash (64 chars)
        record_hash: String,
    },
    /// Attest a record hash as an allowlisted attester (requires attester auth)
    Attest {
        /// Stellar address (G...) of the attester
        attester: String,
        /// Hex-encoded 32-byte record hash
        record_hash: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Revoke all attestations for a record hash (requires admin)
    Revoke {
        /// Hex-encoded 32-byte record hash
        record_hash: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Get the configured attester-registry contract address
    GetAttesterRegistry,
    /// Repoint the attester-registry contract this registry consults (requires admin)
    SetAttesterRegistry {
        /// New attester-registry contract address (C...)
        new_registry: String,
        #[arg(long)]
        source: Option<String>,
    },
}

// ── admin transfer subcommands ────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum AdminSub {
    /// Get the current admin address for a contract
    Get {
        /// Which contract: attester-registry or attestation-registry
        #[arg(long, default_value = "attester-registry")]
        contract: String,
    },
    /// Propose a new admin (requires current admin auth)
    Propose {
        /// New admin address (G...)
        new_admin: String,
        /// Which contract: attester-registry or attestation-registry
        #[arg(long, default_value = "attester-registry")]
        contract: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Accept pending admin transfer (requires pending admin auth)
    Accept {
        /// Which contract: attester-registry or attestation-registry
        #[arg(long, default_value = "attester-registry")]
        contract: String,
        #[arg(long)]
        source: Option<String>,
    },
}

// ── ops (pause/unpause) subcommands ──────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum OpsSub {
    /// Pause a contract (blocks state-changing operations)
    Pause {
        /// Which contract: attester-registry, attestation-registry, or both
        #[arg(long, default_value = "attester-registry")]
        contract: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Unpause a contract
    Unpause {
        /// Which contract: attester-registry, attestation-registry, or both
        #[arg(long, default_value = "attester-registry")]
        contract: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Query whether a contract is currently paused
    IsPaused {
        /// Which contract: attester-registry or attestation-registry
        #[arg(long, default_value = "attester-registry")]
        contract: String,
    },
}

// ── main ──────────────────────────────────────────────────────────────────────

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

    // `config show` reports config problems instead of refusing to print.
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
        Commands::Config { sub } => handle_config(sub, &cli.network, &network_cfg, &cli.config)?,
        Commands::Attester { sub } => handle_attester(sub, &cli.network, &network_cfg)?,
        Commands::Attestation { sub } => handle_attestation(sub, &cli.network, &network_cfg)?,
        Commands::Admin { sub } => handle_admin(sub, &cli.network, &network_cfg)?,
        Commands::Ops { sub } => handle_ops(sub, &cli.network, &network_cfg)?,
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
            println!("This command is a wrapper — for full deploy use:");
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

// ── command handlers ──────────────────────────────────────────────────────────

fn handle_config(
    sub: ConfigSub,
    network: &str,
    cfg: &NetworkConfig,
    config_path: &Option<PathBuf>,
) -> anyhow::Result<()> {
    match sub {
        ConfigSub::Show => {
            let (path, _) = lafiya_config::load_network_config::<PathBuf>(
                network,
                config_path.clone(),
            )?;
            println!("Network: {}", network);
            println!("Config: {:?}", path);
            println!("RPC URL: {}", cfg.rpc_url);
            println!("Passphrase: {}", cfg.network_passphrase);
            println!(
                "Attester registry: {}",
                if cfg.contracts.attester_registry.is_empty() {
                    "<not deployed>".to_string()
                } else {
                    cfg.contracts.attester_registry.clone()
                }
            );
            println!(
                "Attestation registry: {}",
                if cfg.contracts.attestation_registry.is_empty() {
                    "<not deployed>".to_string()
                } else {
                    cfg.contracts.attestation_registry.clone()
                }
            );
            println!("Deployed: {}", cfg.is_deployed());
            println!("Deployment status: {}", deployment_summary(cfg));
            println!(
                "\nSecrets: NEVER stored in networks.toml. Use stellar identities or env vars."
            );
        }
        ConfigSub::List => {} // handled before network resolution
        ConfigSub::Env => {
            println!(
                "# Source this with: eval $(lafiya-cli --network {} config env)",
                network
            );
            println!("export LAFIYA_NETWORK={}", network);
            println!("export LAFIYA_RPC_URL={}", cfg.rpc_url);
            println!(
                "export LAFIYA_NETWORK_PASSPHRASE={:?}",
                cfg.network_passphrase
            );
            println!(
                "export LAFIYA_ATTESTER_REGISTRY_ID={}",
                cfg.contracts.attester_registry
            );
            println!(
                "export LAFIYA_ATTESTATION_REGISTRY_ID={}",
                cfg.contracts.attestation_registry
            );
        }
    }
    Ok(())
}

fn handle_attester(sub: AttesterSub, network: &str, cfg: &NetworkConfig) -> anyhow::Result<()> {
    let contract_id = cfg
        .require_contract_id(network, ContractKind::AttesterRegistry)
        .map_err(|e| anyhow::anyhow!(e))?;

    match sub {
        AttesterSub::Is { address } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            println!("Checking is_attester for {} on {}", address, contract_id);
            println!("RPC: {}", cfg.rpc_url);
            let args = invoke_args(
                cfg,
                contract_id,
                None,
                "is_attester",
                &["--attester", &address],
            );
            println!("> stellar {}", args.join(" "));
            // Read-only query: report a missing CLI without aborting hard.
            if which::which("stellar").is_ok() {
                if let Err(e) = std::process::Command::new("stellar").args(args).status() {
                    eprintln!("Failed to run stellar CLI: {e}. Install with: cargo install --locked stellar-cli");
                }
            } else {
                eprintln!("stellar CLI not found — showing command only. Install with: cargo install --locked stellar-cli");
            }
        }
        AttesterSub::Add { address, source } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "add_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
        AttesterSub::AddWithInfo {
            address,
            license_hash,
            region,
            source,
        } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            if let Some(ref h) = license_hash {
                validate_record_hash("license_hash", h)
                    .context("license_hash must be a 64-char hex string (32 bytes)")?;
            }
            let source = validated_source(source)?;
            let mut fn_args: Vec<String> = vec!["--attester".to_string(), address.clone()];
            match &license_hash {
                Some(h) => {
                    fn_args.push("--license_hash".to_string());
                    fn_args.push(format!("Some({})", h));
                }
                None => {
                    fn_args.push("--license_hash".to_string());
                    fn_args.push("None".to_string());
                }
            }
            match &region {
                Some(r) => {
                    fn_args.push("--region".to_string());
                    fn_args.push(format!("Some({})", r));
                }
                None => {
                    fn_args.push("--region".to_string());
                    fn_args.push("None".to_string());
                }
            }
            let fn_args_ref: Vec<&str> = fn_args.iter().map(String::as_str).collect();
            let args = invoke_args(cfg, contract_id, source.as_deref(), "add_attester_with_info", &fn_args_ref);
            run_stellar(args)?;
        }
        AttesterSub::UpdateInfo {
            address,
            license_hash,
            region,
            source,
        } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            if let Some(ref h) = license_hash {
                validate_record_hash("license_hash", h)
                    .context("license_hash must be a 64-char hex string (32 bytes)")?;
            }
            let source = validated_source(source)?;
            let mut fn_args: Vec<String> = vec!["--attester".to_string(), address.clone()];
            match &license_hash {
                Some(h) => {
                    fn_args.push("--license_hash".to_string());
                    fn_args.push(format!("Some({})", h));
                }
                None => {
                    fn_args.push("--license_hash".to_string());
                    fn_args.push("None".to_string());
                }
            }
            match &region {
                Some(r) => {
                    fn_args.push("--region".to_string());
                    fn_args.push(format!("Some({})", r));
                }
                None => {
                    fn_args.push("--region".to_string());
                    fn_args.push("None".to_string());
                }
            }
            let fn_args_ref: Vec<&str> = fn_args.iter().map(String::as_str).collect();
            let args = invoke_args(cfg, contract_id, source.as_deref(), "update_attester_info", &fn_args_ref);
            run_stellar(args)?;
        }
        AttesterSub::Remove { address, source } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "remove_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
        AttesterSub::Suspend { address, source } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "suspend_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
        AttesterSub::Reinstate { address, source } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "reinstate_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
        AttesterSub::GetInfo { address } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let args = invoke_args(
                cfg,
                contract_id,
                None,
                "get_attester_info",
                &["--attester", &address],
            );
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttesterSub::GetStatus { address } => {
            validate_address("attester address", &address).context("invalid attester address")?;
            let args = invoke_args(
                cfg,
                contract_id,
                None,
                "get_attester_status",
                &["--attester", &address],
            );
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttesterSub::SetMaxAttesters { max, source } => {
            let source = validated_source(source)?;
            let max_str = max.to_string();
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "set_max_attesters",
                &["--max_attesters", &max_str],
            );
            run_stellar(args)?;
        }
        AttesterSub::GetMaxAttesters => {
            let args = invoke_args(cfg, contract_id, None, "get_max_attesters", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttesterSub::GetCount => {
            let args = invoke_args(cfg, contract_id, None, "get_attester_count", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttesterSub::GetSchemaVersion => {
            let args = invoke_args(cfg, contract_id, None, "get_schema_version", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttesterSub::Upgrade {
            new_wasm_hash,
            source,
        } => {
            // wasm hash is 32 bytes = 64 hex chars — reuse record_hash validator
            validate_record_hash("new_wasm_hash", &new_wasm_hash)
                .context("new_wasm_hash must be a 64-char hex string (32 bytes)")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "upgrade",
                &["--new_wasm_hash", &new_wasm_hash],
            );
            run_stellar(args)?;
        }
        AttesterSub::Migrate { source } => {
            let source = validated_source(source)?;
            let args = invoke_args(cfg, contract_id, source.as_deref(), "migrate", &[]);
            run_stellar(args)?;
        }
    }
    Ok(())
}

fn handle_attestation(
    sub: AttestationSub,
    network: &str,
    cfg: &NetworkConfig,
) -> anyhow::Result<()> {
    let contract_id = cfg
        .require_contract_id(network, ContractKind::AttestationRegistry)
        .map_err(|e| anyhow::anyhow!(e))?;

    match sub {
        AttestationSub::Get { record_hash } => {
            validate_record_hash("record_hash", &record_hash)
                .context("invalid record hash (expected a hex encoded 32-byte hash)")?;
            let args = invoke_args(
                cfg,
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
                    "stellar CLI not found — install with cargo install --locked stellar-cli"
                );
            }
        }
        AttestationSub::GetHistory { record_hash } => {
            validate_record_hash("record_hash", &record_hash)
                .context("invalid record hash (expected a hex encoded 32-byte hash)")?;
            let args = invoke_args(
                cfg,
                contract_id,
                None,
                "get_attestation_history",
                &["--record_hash", &record_hash],
            );
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttestationSub::Attest {
            attester,
            record_hash,
            source,
        } => {
            validate_address("attester", &attester).context("invalid attester address")?;
            validate_record_hash("record_hash", &record_hash)
                .context("invalid record hash (expected a hex encoded 32-byte hash)")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "attest",
                &["--attester", &attester, "--record_hash", &record_hash],
            );
            run_stellar(args)?;
        }
        AttestationSub::Revoke { record_hash, source } => {
            validate_record_hash("record_hash", &record_hash)
                .context("invalid record hash (expected a hex encoded 32-byte hash)")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "revoke_attestation",
                &["--record_hash", &record_hash],
            );
            run_stellar(args)?;
        }
        AttestationSub::GetAttesterRegistry => {
            let args = invoke_args(cfg, contract_id, None, "get_attester_registry", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AttestationSub::SetAttesterRegistry {
            new_registry,
            source,
        } => {
            validate_address("new_registry", &new_registry)
                .context("invalid contract address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "set_attester_registry",
                &["--new_registry", &new_registry],
            );
            run_stellar(args)?;
        }
    }
    Ok(())
}

fn handle_admin(sub: AdminSub, network: &str, cfg: &NetworkConfig) -> anyhow::Result<()> {
    let kind = parse_contract_arg(&sub_contract_str(&sub))?;
    let contract_id = cfg
        .require_contract_id(network, kind)
        .map_err(|e| anyhow::anyhow!(e))?;

    match sub {
        AdminSub::Get { .. } => {
            let args = invoke_args(cfg, contract_id, None, "get_admin", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
        AdminSub::Propose {
            new_admin, source, ..
        } => {
            validate_account_address("new_admin", &new_admin)
                .context("invalid new_admin address")?;
            let source = validated_source(source)?;
            let args = invoke_args(
                cfg,
                contract_id,
                source.as_deref(),
                "propose_admin",
                &["--new_admin", &new_admin],
            );
            run_stellar(args)?;
        }
        AdminSub::Accept { source, .. } => {
            let source = validated_source(source)?;
            let args = invoke_args(cfg, contract_id, source.as_deref(), "accept_admin", &[]);
            run_stellar(args)?;
        }
    }
    Ok(())
}

fn handle_ops(sub: OpsSub, network: &str, cfg: &NetworkConfig) -> anyhow::Result<()> {
    let kind = parse_contract_arg(&sub_ops_contract_str(&sub))?;
    let contract_id = cfg
        .require_contract_id(network, kind)
        .map_err(|e| anyhow::anyhow!(e))?;

    match sub {
        OpsSub::Pause { source, .. } => {
            let source = validated_source(source)?;
            let args = invoke_args(cfg, contract_id, source.as_deref(), "pause", &[]);
            run_stellar(args)?;
        }
        OpsSub::Unpause { source, .. } => {
            let source = validated_source(source)?;
            let args = invoke_args(cfg, contract_id, source.as_deref(), "unpause", &[]);
            run_stellar(args)?;
        }
        OpsSub::IsPaused { .. } => {
            let args = invoke_args(cfg, contract_id, None, "is_paused", &[]);
            println!("> stellar {}", args.join(" "));
            if which::which("stellar").is_ok() {
                let _ = std::process::Command::new("stellar").args(args).status();
            } else {
                eprintln!("stellar CLI not found — showing command only.");
            }
        }
    }
    Ok(())
}

// ── small helpers for admin/ops contract routing ──────────────────────────────

fn sub_contract_str(sub: &AdminSub) -> String {
    match sub {
        AdminSub::Get { contract } => contract.clone(),
        AdminSub::Propose { contract, .. } => contract.clone(),
        AdminSub::Accept { contract, .. } => contract.clone(),
    }
}

fn sub_ops_contract_str(sub: &OpsSub) -> String {
    match sub {
        OpsSub::Pause { contract, .. } => contract.clone(),
        OpsSub::Unpause { contract, .. } => contract.clone(),
        OpsSub::IsPaused { contract } => contract.clone(),
    }
}

/// Parse a `--contract` string into a `ContractKind`.
fn parse_contract_arg(s: &str) -> anyhow::Result<ContractKind> {
    match s {
        "attester-registry" | "attester_registry" | "attester" => {
            Ok(ContractKind::AttesterRegistry)
        }
        "attestation-registry" | "attestation_registry" | "attestation" => {
            Ok(ContractKind::AttestationRegistry)
        }
        other => anyhow::bail!(
            "unknown contract '{other}'; expected attester-registry or attestation-registry"
        ),
    }
}

// ── `which` shim (std-only, no extra dep) ────────────────────────────────────

mod which {
    pub fn which(bin: &str) -> Result<std::path::PathBuf, ()> {
        if let Some(paths) = std::env::var_os("PATH") {
            for p in std::env::split_paths(&paths) {
                let full = p.join(bin);
                if full.exists() {
                    return Ok(full);
                }
                #[cfg(windows)]
                {
                    let full_exe = p.join(format!("{}.exe", bin));
                    if full_exe.exists() {
                        return Ok(full_exe);
                    }
                }
            }
        }
        Err(())
    }
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lafiya_config::{ContractIds, NetworkConfig};

    fn test_cfg() -> NetworkConfig {
        NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            contracts: ContractIds {
                attester_registry:
                    "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA".to_string(),
                attestation_registry:
                    "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX".to_string(),
            },
        }
    }

    const CONTRACT_ID: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const ATTESTER_ADDR: &str = "GAHJJJKMOKYE4RVPZEWZTKH5FVI4PA3VL7GK2LFNUBSGBKM7SFGNUQ2";

    // ── invoke_args argv snapshot ─────────────────────────────────────────────

    #[test]
    fn invoke_args_no_source_is_attester() {
        let cfg = test_cfg();
        let args = invoke_args(&cfg, CONTRACT_ID, None, "is_attester", &["--attester", ATTESTER_ADDR]);
        assert_eq!(
            args,
            vec![
                "contract",
                "invoke",
                "--id",
                CONTRACT_ID,
                "--rpc-url",
                "https://soroban-testnet.stellar.org",
                "--network-passphrase",
                "Test SDF Network ; September 2015",
                "--",
                "is_attester",
                "--attester",
                ATTESTER_ADDR,
            ]
        );
    }

    #[test]
    fn invoke_args_with_source() {
        let cfg = test_cfg();
        let args = invoke_args(&cfg, CONTRACT_ID, Some("my-identity"), "add_attester", &["--attester", ATTESTER_ADDR]);
        let source_pos = args.iter().position(|a| a == "--source");
        assert!(source_pos.is_some(), "expected --source in argv");
        assert_eq!(args[source_pos.unwrap() + 1], "my-identity");
    }

    #[test]
    fn invoke_args_empty_fn_args() {
        let cfg = test_cfg();
        let args = invoke_args(&cfg, CONTRACT_ID, None, "get_admin", &[]);
        // "--" separator must precede the function name
        let sep_pos = args.iter().position(|a| a == "--").expect("missing -- separator");
        assert_eq!(args[sep_pos + 1], "get_admin");
        // Nothing after function name
        assert_eq!(args.len(), sep_pos + 2);
    }

    #[test]
    fn invoke_args_rpc_url_and_passphrase_are_separate_elements() {
        let cfg = test_cfg();
        let args = invoke_args(&cfg, CONTRACT_ID, None, "is_paused", &[]);
        // Verify --rpc-url and its value are adjacent separate strings
        let rpc_pos = args.iter().position(|a| a == "--rpc-url").unwrap();
        assert_eq!(args[rpc_pos + 1], "https://soroban-testnet.stellar.org");
        let pp_pos = args.iter().position(|a| a == "--network-passphrase").unwrap();
        assert_eq!(args[pp_pos + 1], "Test SDF Network ; September 2015");
    }

    // ── validated_source ──────────────────────────────────────────────────────

    #[test]
    fn validated_source_none_when_flag_and_env_absent() {
        // Temporarily clear env var (if set)
        std::env::remove_var("STELLAR_ACCOUNT");
        let result = validated_source(None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn validated_source_accepts_valid_identity_name() {
        std::env::remove_var("STELLAR_ACCOUNT");
        // Identity names are short alphanumeric strings
        let result = validated_source(Some("my-identity".to_string())).unwrap();
        assert_eq!(result.as_deref(), Some("my-identity"));
    }

    #[test]
    fn validated_source_rejects_empty_string() {
        std::env::remove_var("STELLAR_ACCOUNT");
        // An empty string is treated as absent (no source)
        let result = validated_source(Some(String::new())).unwrap();
        assert!(result.is_none());
    }

    // ── deployment_summary ────────────────────────────────────────────────────

    #[test]
    fn deployment_summary_not_deployed() {
        let cfg = NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            contracts: ContractIds {
                attester_registry: String::new(),
                attestation_registry: String::new(),
            },
        };
        let s = deployment_summary(&cfg);
        assert!(s.contains("not yet deployed"), "{s}");
    }

    #[test]
    fn deployment_summary_deployed_contains_ids() {
        let cfg = test_cfg();
        let s = deployment_summary(&cfg);
        assert!(s.contains("deployed"), "{s}");
        assert!(s.contains("CBCRV4"), "{s}");
        assert!(s.contains("CCWPKE"), "{s}");
    }

    #[test]
    fn deployment_summary_partial_names_missing() {
        let cfg = NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            contracts: ContractIds {
                attester_registry:
                    "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA".to_string(),
                attestation_registry: String::new(),
            },
        };
        let s = deployment_summary(&cfg);
        assert!(s.contains("partially"), "{s}");
        assert!(s.contains("attestation_registry"), "{s}");
    }

    // ── DeployMode ────────────────────────────────────────────────────────────

    #[test]
    fn deploy_mode_new() {
        assert_eq!(DeployMode::new(true, false), DeployMode::BuildOnly);
        assert_eq!(DeployMode::new(false, true), DeployMode::DryRun);
        assert_eq!(DeployMode::new(false, false), DeployMode::Deploy);
        // build_only takes precedence
        assert_eq!(DeployMode::new(true, true), DeployMode::BuildOnly);
    }

    // ── DeployIdentity ────────────────────────────────────────────────────────

    #[test]
    fn deploy_identity_build_only_does_not_require_admin_or_source() {
        let id = DeployIdentity::resolve(None, None, None, None, DeployMode::BuildOnly).unwrap();
        assert!(id.admin.is_none());
        assert!(id.source.is_none());
    }

    #[test]
    fn deploy_identity_deploy_requires_admin() {
        let err = DeployIdentity::resolve(None, Some("alice".to_string()), None, None, DeployMode::Deploy)
            .unwrap_err()
            .to_string();
        assert!(err.contains("admin"), "{err}");
    }

    #[test]
    fn deploy_identity_deploy_requires_source() {
        let err = DeployIdentity::resolve(
            Some(ATTESTER_ADDR.to_string()),
            None,
            None,
            None,
            DeployMode::Deploy,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("source"), "{err}");
    }

    #[test]
    fn deploy_identity_env_fallback() {
        std::env::remove_var("STELLAR_ACCOUNT");
        std::env::remove_var("ADMIN_ADDRESS");
        let id = DeployIdentity::resolve(
            None,
            None,
            Some(ATTESTER_ADDR.to_string()),
            Some("alice".to_string()),
            DeployMode::Deploy,
        )
        .unwrap();
        assert_eq!(id.admin.as_deref(), Some(ATTESTER_ADDR));
        assert_eq!(id.source.as_deref(), Some("alice"));
    }

    // ── parse_contract_arg ────────────────────────────────────────────────────

    #[test]
    fn parse_contract_arg_variants() {
        assert_eq!(
            parse_contract_arg("attester-registry").unwrap(),
            ContractKind::AttesterRegistry
        );
        assert_eq!(
            parse_contract_arg("attestation-registry").unwrap(),
            ContractKind::AttestationRegistry
        );
        assert_eq!(
            parse_contract_arg("attester").unwrap(),
            ContractKind::AttesterRegistry
        );
        assert_eq!(
            parse_contract_arg("attestation").unwrap(),
            ContractKind::AttestationRegistry
        );
        assert!(parse_contract_arg("unknown").is_err());
    }

    // ── clap missing-argument errors ──────────────────────────────────────────

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

    #[test]
    fn attester_upgrade_missing_wasm_hash_names_the_argument() {
        let err = Cli::try_parse_from(["lafiya-cli", "attester", "upgrade"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("NEW_WASM_HASH"),
            "expected error to name the missing `new_wasm_hash` argument, got: {message}"
        );
    }
}
