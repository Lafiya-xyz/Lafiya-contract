//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.
//!
//! Every operator supplied value (network name, address, contract id, record
//! hash, admin/source account) is validated locally before the stellar CLI is
//! invoked, so malformed input fails fast with an actionable message.

mod audit;
mod auth_decode;
mod logging;

use anyhow::Context;
use clap::{Parser, Subcommand};
use lafiya_config::{
    load_networks, resolve_network, validate_account_address, validate_address,
    validate_network_name, validate_record_hash, validate_source_account, ContractKind,
    DeploymentState, NetworkConfig,
};
use lafiya_rpc_resilience::{
    backoff_with_jitter, http::HttpRpcProvider, FailoverClient, RecoveryLog, RecoveryResult,
    RetryPolicy, RpcProvider, SignedTx,
};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod interface;
mod deployment_ledger;
use deployment_ledger::{DeployEvent, DeploymentRecord};

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

    /// Log output format (logs go to stderr)
    #[arg(long, value_enum, default_value = "text", global = true)]
    log_format: logging::LogFormat,

    /// Increase log verbosity (-v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Verify published trust data (SEP-1 stellar.toml)
    Trust {
        #[command(subcommand)]
        sub: TrustSub,
    },
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
    /// Decode authorization entries for signer review (ADR-0007)
    Auth {
        #[command(subcommand)]
        sub: AuthSub,
    },
    /// Inspect the local operation audit log
    Audit {
        #[command(subcommand)]
        sub: AuditSub,
    },
    /// Negotiate the interface of a deployed contract via `get_interface`
    Interface {
        /// Which contract to query
        #[arg(value_enum)]
        contract: InterfaceTarget,
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
    /// Append-only per-network deployment history (deployments/<network>.jsonl)
    Deployments {
        #[command(subcommand)]
        sub: DeploymentsSub,
    },
}

#[derive(Subcommand, Debug)]
enum DeploymentsSub {
    /// Append one event record to deployments/<network>.jsonl.
    ///
    /// Called automatically by scripts/deploy.sh and scripts/upgrade.sh after
    /// on-chain confirmation; can also be run by hand for an out-of-band
    /// change (e.g. admin_transfer).
    Record {
        /// deploy | initialize | upgrade | migrate | admin_transfer | repoint
        #[arg(long)]
        event: String,
        /// attester-registry | attestation-registry | multisig-account
        #[arg(long)]
        contract_kind: String,
        /// The affected contract's Stellar contract ID (C...)
        #[arg(long)]
        contract_id: String,
        /// Path to the wasm now running, to compute wasm_sha256 (omit for
        /// events that don't change code, e.g. admin_transfer)
        #[arg(long)]
        wasm: Option<PathBuf>,
        /// sha256 of the wasm this event replaced (upgrade/migrate)
        #[arg(long)]
        previous_wasm_sha256: Option<String>,
        /// Confirmed transaction hash. Omit only if it genuinely could not
        /// be captured -- the record is still appended with tx_hash: null,
        /// but `deployments verify` cannot confirm it against the chain.
        #[arg(long)]
        tx_hash: Option<String>,
        /// Ledger sequence the transaction closed in, if known
        #[arg(long)]
        ledger: Option<u32>,
        /// Identity/signer-set that authorized this event
        #[arg(long)]
        operator: String,
    },
    /// Verify a network's ledger file: hash chain intact, ledgers
    /// monotonic, required fields present. Offline check only -- does not
    /// query the chain (see deployments/README.md).
    Verify,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum InterfaceTarget {
    Attester,
    Attestation,
}

#[derive(Subcommand, Debug)]
enum TrustSub {
    /// Fetch https://<domain>/.well-known/stellar.toml and compare its contract ids and
    /// wasm hashes with the local config and on-chain instance data
    Verify {
        /// Domain serving the stellar.toml, e.g. lafiya.xyz
        #[arg(long)]
        domain: String,
        /// PEM root certificate to trust instead of the WebPKI roots (staging hosts)
        #[arg(long)]
        ca_cert: Option<PathBuf>,
        /// Skip the on-chain wasm hash comparison
        #[arg(long, default_value_t = false)]
        skip_chain: bool,
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
    /// Print the JSON Schema for networks.toml (committed as config/networks.schema.json)
    Schema,
}

fn parse_override(raw: &str) -> Result<(String, String), String> {
    raw.split_once('=')
        .map(|(k, v)| (k.trim().to_string(), v.to_string()))
        .ok_or_else(|| format!("expected KEY=VALUE, got `{raw}`"))
}

/// Single-quote a value for safe use in `eval`.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
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
enum AuthSub {
    /// Render a SorobanAuthorizationEntry or TransactionEnvelope (base64 XDR,
    /// or a path to a file containing it) as a human-readable tree
    Decode {
        /// Base64 XDR, or a path to a file containing it
        input: String,
        /// Output format
        #[arg(long, value_enum, default_value = "text")]
        format: DecodeFormat,
        /// Extra known address label, as ADDRESS=NAME (repeatable)
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Current ledger sequence, to show how soon the entry expires
        #[arg(long)]
        current_ledger: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum DecodeFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
enum AuditSub {
    /// Print the audit log (LAFIYA_AUDIT_LOG or ~/.lafiya/audit.jsonl)
    Show,
    /// Verify the audit log hash chain
    Verify,
}

#[derive(Subcommand, Debug)]
enum AttestationSub {
    /// Get attestation for a record hash (hex encoded 32-byte hash)
    Get {
        /// Hex string of 32-byte record hash (64 chars)
        record_hash: String,
    },
    /// Compute the versioned, domain-separated hash of a local record JSON file
    Hash {
        /// Path to the record JSON file
        record: PathBuf,
    },
    /// Verify a local record JSON file against its on-chain attestation
    Verify {
        /// Path to the record JSON file
        record: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _log_guard = logging::init(cli.log_format, cli.verbose);

    // The audit log is local and needs no network config.
    if let Commands::Audit { sub } = &cli.command {
        return run_audit(sub);
    }

    if let Commands::Config {
        sub: ConfigSub::Schema,
    } = &cli.command
    {
        print!("{}", lafiya_config::json_schema());
        return Ok(());
    }

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

    let resolved = resolve_network(
        &cli.network,
        cli.config.as_deref(),
        |var| std::env::var(var).ok(),
        &cli.overrides,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let network_cfg = resolved.config.clone();

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
        Commands::Audit { .. } => {} // handled above
        Commands::Auth {
            sub:
                AuthSub::Decode {
                    input,
                    format,
                    labels,
                    current_ledger,
                },
        } => {
            let _span = tracing::info_span!("auth.decode", network = %cli.network).entered();
            let raw = match std::fs::read_to_string(&input) {
                Ok(contents) => contents,
                Err(_) => input,
            };
            let ctx = auth_decode::DecodeContext {
                network_name: cli.network.clone(),
                network_passphrase: network_cfg.network_passphrase.clone(),
                labels: known_labels(&network_cfg, &labels)?,
                current_ledger,
            };
            let entries = auth_decode::decode_input(&raw, &ctx)?;
            match format {
                DecodeFormat::Text => print!("{}", auth_decode::render_text(&entries, &ctx)),
                DecodeFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
            }
        }
        Commands::Config { sub } => {
            match sub {
                ConfigSub::Show => {
                    let from = |key: &str| format!("  (from {})", resolved.sources[key]);
                    let or_undeployed = |id: &str| {
                        if id.is_empty() {
                            "<not deployed>".to_string()
                        } else {
                            id.to_string()
                        }
                    };
                    println!("Network: {}", cli.network);
                    println!("Config: {:?}", resolved.config_path);
                    println!("RPC URL: {}{}", network_cfg.rpc_url, from("rpc_url"));
                    if !network_cfg.rpc_urls.is_empty() {
                        println!(
                            "RPC failover order: {}{}",
                            network_cfg.rpc_urls.join(", "),
                            from("rpc_urls")
                        );
                    }
                    println!(
                        "Passphrase: {}{}",
                        network_cfg.network_passphrase,
                        from("network_passphrase")
                    );
                    println!(
                        "Attester registry: {}{}",
                        or_undeployed(&network_cfg.contracts.attester_registry),
                        from("contracts.attester_registry")
                    );
                    println!(
                        "Attestation registry: {}{}",
                        or_undeployed(&network_cfg.contracts.attestation_registry),
                        from("contracts.attestation_registry")
                    );
                    println!("Deployed: {}", network_cfg.is_deployed());
                    println!("Deployment status: {}", deployment_summary(&network_cfg));
                    println!("\nSecrets: NEVER stored in networks.toml. Use stellar identities or env vars.");
                }
                ConfigSub::List | ConfigSub::Schema => {} // handled above
                ConfigSub::Env => {
                    println!(
                        "# Source this with: eval \"$(lafiya-cli --network {} config env)\"",
                        cli.network
                    );
                    for (var, value) in [
                        ("LAFIYA_NETWORK", cli.network.as_str()),
                        (
                            "LAFIYA_CONFIG_PATH",
                            &resolved.config_path.display().to_string(),
                        ),
                        ("LAFIYA_RPC_URL", &network_cfg.rpc_url),
                        ("LAFIYA_NETWORK_PASSPHRASE", &network_cfg.network_passphrase),
                        (
                            "LAFIYA_ATTESTER_REGISTRY_ID",
                            &network_cfg.contracts.attester_registry,
                        ),
                        (
                            "LAFIYA_ATTESTATION_REGISTRY_ID",
                            &network_cfg.contracts.attestation_registry,
                        ),
                    ] {
                        println!("export {var}={}", shell_quote(value));
                    }
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
                run_audited(
                    "attester add",
                    &cli.network,
                    contract_id,
                    "add_attester",
                    &[&address],
                    source.as_deref(),
                    args,
                )?;
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
                run_audited(
                    "attester remove",
                    &cli.network,
                    contract_id,
                    "remove_attester",
                    &[&address],
                    source.as_deref(),
                    args,
                )?;
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
            AttestationSub::Hash { record } => {
                let content = std::fs::read_to_string(&record)?;
                let rec: lafiya_config::record::EmergencyRecord = serde_json::from_str(&content)?;
                let hash_bytes = rec.hash()?;
                let hash_hex: String = hash_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                println!("Canonicalized JSON: {}", rec.canonicalize()?);
                println!("Computed Hash (hex): {}", hash_hex);
            }
            AttestationSub::Verify { record } => {
                if network_cfg.contracts.attestation_registry.is_empty() {
                    anyhow::bail!("attestation_registry not deployed for '{}'", cli.network);
                }
                let content = std::fs::read_to_string(&record)?;
                let rec: lafiya_config::record::EmergencyRecord = serde_json::from_str(&content)?;
                let hash_bytes = rec.hash()?;
                let hash_hex: String = hash_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                println!("Verifying record hash: {}", hash_hex);
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
                    &hash_hex,
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
        Commands::Interface { contract } => {
            let (kind, req) = match contract {
                InterfaceTarget::Attester => {
                    (ContractKind::AttesterRegistry, interface::ATTESTER_REGISTRY)
                }
                InterfaceTarget::Attestation => (
                    ContractKind::AttestationRegistry,
                    interface::ATTESTATION_REGISTRY,
                ),
            };
            let contract_id = network_cfg
                .require_contract_id(&cli.network, kind)
                .map_err(|e| anyhow::anyhow!(e))?;
            if which::which("stellar").is_err() {
                anyhow::bail!(
                    "stellar CLI not found - install with cargo install --locked stellar-cli"
                );
            }
            let args = invoke_args(&network_cfg, contract_id, None, "get_interface", &[]);
            let output = std::process::Command::new("stellar").args(args).output()?;
            let raw = output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned());
            let info = interface::negotiate(contract_id, raw.as_deref(), req)
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("Contract: {contract_id}");
            println!("Kind: {}", info.contract_kind);
            println!("Interface version: {}", info.interface_version);
            println!("Schema version: {}", info.schema_version);
            println!("Event version: {}", info.event_version);
            println!("Features: {}", info.features.join(", "));
        }
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
        Commands::Deployments { sub } => match sub {
            DeploymentsSub::Record {
                event,
                contract_kind,
                contract_id,
                wasm,
                previous_wasm_sha256,
                tx_hash,
                ledger,
                operator,
            } => {
                let event: DeployEvent = event
                    .parse()
                    .map_err(|e: String| anyhow::anyhow!(e))?;
                let wasm_sha256 = wasm
                    .as_deref()
                    .map(wasm_sha256_hex)
                    .transpose()
                    .context("failed to hash --wasm")?;
                if tx_hash.is_none() {
                    eprintln!(
                        "WARNING: no --tx-hash supplied; recording with tx_hash: null. \
                         `deployments verify` cannot confirm this record against the chain."
                    );
                }
                let record = DeploymentRecord {
                    event,
                    contract_kind,
                    contract_id,
                    wasm_sha256,
                    previous_wasm_sha256,
                    tx_hash,
                    ledger,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    git_commit: git_commit_hash().unwrap_or_else(|_| "unknown".to_string()),
                    release_version: env!("CARGO_PKG_VERSION").to_string(),
                    operator,
                    prev_record_sha256: None,
                };
                let dir = deployments_dir();
                let path = deployment_ledger::append(&dir, &cli.network, record)
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!("Appended deployment record to {}", path.display());
            }
            DeploymentsSub::Verify => {
                let dir = deployments_dir();
                let report = deployment_ledger::verify(&dir, &cli.network)
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!(
                    "Checked {} record(s) for network {}",
                    report.records_checked, cli.network
                );
                if report.is_ok() {
                    println!("OK: ledger is internally consistent.");
                } else {
                    for err in &report.errors {
                        eprintln!("ERROR: {err}");
                    }
                    anyhow::bail!("{} error(s) found in deployment ledger", report.errors.len());
                }
            }
        },
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
/// Run a mutating stellar CLI invocation and return the submitted
/// transaction hash when the CLI reports one. Stderr is streamed through so
/// the operator still sees progress.
fn run_stellar(args: Vec<String>) -> anyhow::Result<Option<String>> {
    tracing::info!(command = %format!("stellar {}", args.join(" ")), "invoking stellar CLI");
    if which::which("stellar").is_err() {
        anyhow::bail!("stellar CLI not found");
    }
    let started = std::time::Instant::now();
    let mut child = std::process::Command::new("stellar")
        .args(args)
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut tx_hash = None;
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            eprintln!("{}", logging::redact(&line));
            if tx_hash.is_none() && line.to_ascii_lowercase().contains("transaction") {
                tx_hash = line
                    .split(|c: char| !c.is_ascii_hexdigit())
                    .find(|w| w.len() == 64)
                    .map(str::to_string);
            }
        }
    }
    let status = child.wait()?;
    tracing::info!(
        latency_ms = started.elapsed().as_millis() as u64,
        success = status.success(),
        tx_hash = tx_hash.as_deref().unwrap_or(""),
        "stellar CLI finished"
    );
    if !status.success() {
        anyhow::bail!("stellar CLI failed");
    }
    Ok(tx_hash)
}

/// Run a mutating command inside a tracing span and append its outcome to
/// the local audit log, whether it succeeded or failed.
fn run_audited(
    command: &str,
    network: &str,
    contract: &str,
    function: &str,
    call_args: &[&str],
    signer: Option<&str>,
    stellar_args: Vec<String>,
) -> anyhow::Result<()> {
    let span = tracing::info_span!("operation", command, network, contract, function);
    let _entered = span.enter();
    let result = run_stellar(stellar_args);
    let outcome = match &result {
        Ok(_) => "success".to_string(),
        Err(e) => format!("failure: {e}"),
    };
    let op = audit::Operation {
        command,
        network,
        contract,
        function,
        args: call_args,
        signer,
        tx_hash: result.as_ref().ok().and_then(|h| h.as_deref()),
        outcome: &outcome,
    };
    let path = audit::default_path();
    match audit::append(&path, &op) {
        Ok(r) => tracing::info!(seq = r.seq, path = %path.display(), "audit record written"),
        Err(e) => {
            tracing::error!(error = %e, path = %path.display(), "failed to write audit record")
        }
    }
    result.map(|_| ())
}

fn run_audit(sub: &AuditSub) -> anyhow::Result<()> {
    let path = audit::default_path();
    let records = audit::read_all(&path)?;
    match sub {
        AuditSub::Show => {
            for r in &records {
                println!("{}", serde_json::to_string(r)?);
            }
            if records.is_empty() {
                eprintln!("no audit records in {}", path.display());
            }
        }
        AuditSub::Verify => match audit::verify(&records) {
            Ok(n) => println!("OK: {n} records verified in {}", path.display()),
            Err(e) => anyhow::bail!("audit log {} is broken: {e}", path.display()),
        },
    }
    Ok(())
}

/// Known address labels: deployed contracts from config plus `--label` flags.
fn known_labels(cfg: &NetworkConfig, extra: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut labels = BTreeMap::new();
    for (id, name) in [
        (&cfg.contracts.attester_registry, "lafiya attester-registry"),
        (
            &cfg.contracts.attestation_registry,
            "lafiya attestation-registry",
        ),
    ] {
        if !id.is_empty() {
            labels.insert(id.clone(), name.to_string());
        }
    }
    for pair in extra {
        let (addr, name) = pair
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--label must be ADDRESS=NAME, got '{pair}'"))?;
        labels.insert(addr.trim().to_string(), name.trim().to_string());
    }
    Ok(labels)
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

/// `<repo-root>/deployments`, found the same way `lafiya_config::default_config_path`
/// locates `config/networks.toml`: relative to the current working directory.
fn deployments_dir() -> PathBuf {
    PathBuf::from("deployments")
}

fn wasm_sha256_hex(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).with_context(|| format!("reading {path:?}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn git_commit_hash() -> anyhow::Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .context("running git rev-parse HEAD")?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse HEAD failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
    fn set_flag_parses_key_value_pairs() {
        let cli = Cli::try_parse_from([
            "lafiya-cli",
            "--set",
            "rpc_url=https://a.example/?x=1",
            "config",
            "show",
        ])
        .unwrap();
        assert_eq!(
            cli.overrides,
            vec![("rpc_url".to_string(), "https://a.example/?x=1".to_string())]
        );
        assert!(Cli::try_parse_from(["lafiya-cli", "--set", "rpc_url", "config", "show"]).is_err());
    }

    #[test]
    fn with_rpc_url_replaces_the_endpoint() {
        let args: Vec<String> = ["contract", "invoke", "--rpc-url", "https://a", "--", "f"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            with_rpc_url(&args, "https://b"),
            ["contract", "invoke", "--rpc-url", "https://b", "--", "f"]
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a ; b"), "'a ; b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

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
