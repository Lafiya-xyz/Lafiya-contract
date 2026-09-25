//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.
//!
//! Every operator supplied value (network name, address, contract id, record
//! hash, admin/source account) is validated locally before the stellar CLI is
//! invoked, so malformed input fails fast with an actionable message.

mod trust;

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
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

    /// Override one config value for this invocation, e.g. `--set rpc_url=https://...`.
    /// Takes precedence over LAFIYA_<NETWORK>_<KEY> env vars and networks.local.toml.
    #[arg(long = "set", value_name = "KEY=VALUE", global = true, value_parser = parse_override)]
    overrides: Vec<(String, String)>,

    /// Print the RPC recovery log of every submitted transaction
    #[arg(short, long, global = true)]
    verbose: bool,

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
enum AttestationSub {
    /// Get attestation for a record hash (hex encoded 32-byte hash)
    Get {
        /// Hex string of 32-byte record hash (64 chars)
        record_hash: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

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
                submit_invocation(&network_cfg, args, source, cli.verbose)?;
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
                submit_invocation(&network_cfg, args, source, cli.verbose)?;
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
        Commands::Trust {
            sub:
                TrustSub::Verify {
                    domain,
                    ca_cert,
                    skip_chain,
                },
        } => {
            let ca_pem = ca_cert
                .map(|p| std::fs::read(&p).with_context(|| format!("reading {}", p.display())))
                .transpose()?;
            println!("Fetching {}", trust::stellar_toml_url(&domain));
            let published = trust::parse(&trust::fetch(&domain, ca_pem.as_deref())?)?;
            let (matched, mut problems) =
                trust::compare_with_config(&published, &cli.network, &network_cfg);
            for entry in &matched {
                println!(
                    "OK   {} contract id {} matches local config",
                    entry.name, entry.contract_id
                );
                if skip_chain {
                    continue;
                }
                let mut onchain = Err(anyhow::anyhow!("no RPC endpoint configured"));
                for url in network_cfg.rpc_endpoints() {
                    onchain = trust::onchain_wasm_hash(url, &entry.contract_id);
                    if onchain.is_ok() {
                        break;
                    }
                }
                match onchain {
                    Ok(hash) if hash.eq_ignore_ascii_case(&entry.wasm_hash) => {
                        println!("OK   {} on-chain wasm hash {hash}", entry.name)
                    }
                    Ok(hash) => problems.push(format!(
                        "{}: stellar.toml wasm hash {} but on-chain instance runs {hash}",
                        entry.name, entry.wasm_hash
                    )),
                    Err(e) => problems.push(format!("{}: {e:#}", entry.name)),
                }
            }
            for problem in &problems {
                eprintln!("FAIL {problem}");
            }
            if !problems.is_empty() {
                anyhow::bail!(
                    "stellar.toml for {domain} has {} mismatch(es)",
                    problems.len()
                );
            }
            println!(
                "stellar.toml for {domain} matches network '{}'",
                cli.network
            );
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
        cfg.rpc_endpoints()[0].to_string(),
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

/// Run `stellar` with `args`, feeding `stdin`, and return its trimmed stdout.
fn stellar_output(args: &[String], stdin: Option<&str>) -> anyhow::Result<String> {
    if which::which("stellar").is_err() {
        anyhow::bail!("stellar CLI not found");
    }
    let mut child = Command::new("stellar")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!("stellar {} failed", args.first().map_or("", |a| a.as_str()));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// `args` with every `--rpc-url` value replaced by `url`.
fn with_rpc_url(args: &[String], url: &str) -> Vec<String> {
    let mut args = args.to_vec();
    if let Some(i) = args.iter().position(|a| a == "--rpc-url") {
        args[i + 1] = url.to_string();
    }
    args
}

/// Build, simulate and sign a `stellar contract invoke` transaction, then
/// submit it through [`FailoverClient`] across every configured RPC endpoint.
///
/// Building and simulating are read-only, so they simply move on to the next
/// endpoint on failure. Submission uses poll-before-retry recovery (ADR-0011):
/// an ambiguous failure is never "fixed" by sending a new transaction.
fn submit_invocation(
    cfg: &NetworkConfig,
    invoke: Vec<String>,
    source: Option<String>,
    verbose: bool,
) -> anyhow::Result<()> {
    let source = source
        .or_else(|| first_non_empty(std::env::var(ENV_SOURCE).ok(), None))
        .ok_or_else(|| {
            anyhow::anyhow!("signing requires a transaction source: pass --source <identity> or set {ENV_SOURCE}")
        })?;
    let passphrase = cfg.network_passphrase.clone();
    let endpoints = cfg.rpc_endpoints();

    let mut build = invoke.clone();
    let separator = build.iter().position(|a| a == "--").unwrap_or(build.len());
    build.insert(separator, "--build-only".to_string());
    if !build.iter().any(|a| a == "--source") {
        build.splice(
            separator..separator,
            ["--source".to_string(), source.clone()],
        );
    }
    println!("> stellar {}", build.join(" "));

    let mut simulated = Err(anyhow::anyhow!("no RPC endpoint configured"));
    for url in &endpoints {
        simulated = stellar_output(&with_rpc_url(&build, url), None).and_then(|unsigned| {
            let simulate = [
                "tx",
                "simulate",
                "--source-account",
                &source,
                "--rpc-url",
                url,
                "--network-passphrase",
                &passphrase,
            ]
            .map(String::from);
            stellar_output(&simulate, Some(&unsigned))
        });
        match &simulated {
            Ok(_) => break,
            Err(e) => eprintln!("build/simulate via {url} failed: {e}"),
        }
    }
    let simulated = simulated?;

    let sign = [
        "tx",
        "sign",
        "--sign-with-key",
        &source,
        "--rpc-url",
        endpoints[0],
        "--network-passphrase",
        &passphrase,
    ]
    .map(String::from);
    let envelope = stellar_output(&sign, Some(&simulated))?;
    let hash_args = [
        "tx",
        "hash",
        "--rpc-url",
        endpoints[0],
        "--network-passphrase",
        &passphrase,
    ]
    .map(String::from);
    let hash = stellar_output(&hash_args, Some(&envelope))?;

    let providers: Vec<Box<dyn RpcProvider>> = endpoints
        .iter()
        .map(|url| Box::new(HttpRpcProvider::new(*url)) as Box<dyn RpcProvider>)
        .collect();
    let policy = RetryPolicy {
        // Leave time for the transaction to land in a ledger (~5s each).
        max_poll_rounds: 10,
        ..RetryPolicy::default()
    };
    let mut client = FailoverClient::new(providers, policy)
        .with_backoff(backoff_with_jitter)
        .with_sleep(std::thread::sleep);
    let mut log = RecoveryLog::new();
    let result = client.submit_with_recovery(&SignedTx::new(&hash, envelope), &mut log);
    if verbose {
        for line in log.lines() {
            eprintln!("[rpc] {line}");
        }
    }

    match result {
        RecoveryResult::Accepted {
            ledger, provider, ..
        } => {
            println!("Transaction {hash} succeeded in ledger {ledger} (via {provider})");
            Ok(())
        }
        RecoveryResult::RejectedOnChain { reason } => {
            anyhow::bail!("transaction {hash} was rejected ({reason}); do not resubmit it")
        }
        RecoveryResult::ExhaustedNeedsOperator { last_known } => anyhow::bail!(
            "outcome of transaction {hash} is unknown ({last_known:?}) after retrying every RPC endpoint. \
             Check it with `stellar tx fetch --hash {hash}` before retrying; see docs/runbooks/rpc-outage-recovery.md"
        ),
    }
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
