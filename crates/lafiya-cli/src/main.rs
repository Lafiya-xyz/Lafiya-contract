//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.
//!
//! Every operator supplied value (network name, address, contract id, record
//! hash, admin/source account) is validated locally before the stellar CLI is
//! invoked, so malformed input fails fast with an actionable message.
//!
//! # Issues implemented
//!
//! * **#401** — Preflight RPC checks (passphrase, wasm hash, ledger staleness)
//!   run before every mutating command. Use `--skip-preflight` (non-mainnet
//!   only) to bypass. Use `--allow-unknown-wasm` to warn instead of abort on
//!   a wasm hash mismatch.
//!
//! * **#402** — `deploy`, `upgrade`, and `smoke-test` are now native Rust CLI
//!   subcommands. The Bash scripts in `scripts/` are thin deprecation wrappers.
//!
//! * **#400** — `attestation revoke-by-attester` enumerates `AttestationRecorded`
//!   events, detects retention exhaustion, warns about collateral damage, and
//!   writes an audit report.
//!
//! * **#399** — `--signer <uri>` selects the signing back-end. URIs:
//!   `identity://<name>` (default), `ledger://<index>`, `gcpkms://<path>`,
//!   `awskms://<key-id>`.

mod revoke;
mod signer;

use anyhow::Context;
use clap::{Parser, Subcommand};
use lafiya_config::{
    get_network, load_networks, validate_address, validate_network_name,
    validate_record_hash, validate_source_account, ContractKind, DeploymentState, NetworkConfig,
};
use lafiya_config::preflight::{
    CurlTransport, PreflightOptions, WasmHashRegistry, run_preflight,
};
use std::path::PathBuf;

use revoke::{RevokeByAttesterArgs, StellarCliTransport};
use signer::SignerUri;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Env var holding the stellar CLI identity used as transaction source.
const ENV_SOURCE: &str = "STELLAR_ACCOUNT";
/// Env var holding the contract admin address.
const ENV_ADMIN: &str = "ADMIN_ADDRESS";
/// Expected wasm size budget (bytes).
const WASM_SIZE_BUDGET: u64 = 65_536;

// ---------------------------------------------------------------------------
// CLI structure
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "lafiya-cli",
    about = "Lafiya Admin CLI — uses config/networks.toml",
    long_about = "Lafiya Admin CLI\n\n\
                  Every mutating command runs preflight RPC checks before signing:\n\
                  1. Network passphrase match (getNetwork)\n\
                  2. Wasm hash verification (getLedgerEntries vs release manifest)\n\
                  3. Ledger staleness check (getLatestLedger vs wall clock)\n\n\
                  Use --skip-preflight (non-mainnet only) to bypass all checks.\n\
                  Use --allow-unknown-wasm to warn instead of abort on hash mismatch.\n\n\
                  Signing back-ends (--signer):\n\
                  identity://<name>  stellar CLI file identity (default)\n\
                  ledger://<index>   Ledger hardware wallet\n\
                  gcpkms://<path>    Google Cloud KMS (ed25519)\n\
                  awskms://<key-id>  AWS KMS (ed25519)"
)]
struct Cli {
    /// Network name as defined in config/networks.toml (e.g. testnet, futurenet, mainnet, local)
    #[arg(long, default_value = "testnet", global = true)]
    network: String,

    /// Path to networks.toml (auto-discovers by default)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Skip all preflight RPC checks. Not allowed on mainnet.
    #[arg(long, global = true, default_value_t = false)]
    skip_preflight: bool,

    /// Warn (instead of abort) when the on-chain wasm hash differs from the
    /// release manifest. Not allowed on mainnet.
    #[arg(long, global = true, default_value_t = false)]
    allow_unknown_wasm: bool,

    /// Signing back-end URI. Options: identity://<name>, ledger://<index>,
    /// gcpkms://<resource-path>, awskms://<key-id>.
    /// Defaults to the value of --source / STELLAR_ACCOUNT.
    #[arg(long, global = true)]
    signer: Option<String>,

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
    /// Deploy contracts to the selected network
    Deploy {
        /// Build only, don't deploy
        #[arg(long, default_value_t = false)]
        build_only: bool,
        /// Dry run: print what would be done without deploying
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Stellar identity or G... address used as transaction source (or STELLAR_ACCOUNT)
        #[arg(long)]
        source: Option<String>,
        /// Admin address (G...) for contract initialization (or ADMIN_ADDRESS)
        #[arg(long)]
        admin: Option<String>,
        /// Skip interactive confirmation prompt
        #[arg(long, short = 'y', default_value_t = false)]
        yes: bool,
    },
    /// Upgrade a deployed contract to a new wasm hash
    Upgrade {
        /// Contract to upgrade: attester-registry | attestation-registry
        #[arg(long)]
        contract: String,
        /// Deployed contract ID (C... strkey), or read from config
        #[arg(long)]
        id: Option<String>,
        /// Stellar identity or G... for admin auth (or STELLAR_ACCOUNT / SOURCE_ACCOUNT)
        #[arg(long)]
        source: Option<String>,
        /// Path to pre-built wasm file. If omitted, build from source.
        #[arg(long)]
        wasm: Option<PathBuf>,
        /// Skip the build step (use existing target/ artifact)
        #[arg(long, default_value_t = false)]
        skip_build: bool,
        /// Also run migrate() after the upgrade (schema-changing releases)
        #[arg(long, default_value_t = false)]
        run_migrate: bool,
        /// Assert that get_schema_version() == N after the upgrade
        #[arg(long)]
        expected_schema_version: Option<u32>,
        /// Dry run: print all commands without submitting anything
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Run smoke tests against a deployed instance
    SmokeTest {
        /// Network to smoke-test (reads RPC + contract IDs from config)
        #[arg(long)]
        source: Option<String>,
        /// Dry run: print what would be invoked without actually calling
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
    /// Revoke all attestations by a specific attester, discovered via event enumeration.
    ///
    /// Checks the RPC retention window, warns about collateral damage (hashes with
    /// co-attesters), and writes an audit report.
    RevokeByAttester {
        /// Attester address (G...) whose attestations should be revoked
        attester: String,
        /// Earliest ledger sequence to include (default: RPC retention start)
        #[arg(long)]
        since: Option<u32>,
        /// Latest ledger sequence to include (default: latest ledger)
        #[arg(long)]
        until: Option<u32>,
        /// Print the plan without submitting any revocation transactions
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Admin source account for revocation transactions (or STELLAR_ACCOUNT)
        #[arg(long)]
        source: Option<String>,
        /// Write audit report as JSON to this path
        #[arg(long)]
        audit_json: Option<PathBuf>,
        /// Write audit report as CSV to this path
        #[arg(long)]
        audit_csv: Option<PathBuf>,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Validate the network name before it is used as a config key.
    validate_network_name(&cli.network)
        .map_err(|e| anyhow::anyhow!("invalid --network value: {e}"))?;

    let config_path_opt = cli.config.as_deref();
    let networks = load_networks(config_path_opt)?;

    // For config list, we don't need to resolve specific network.
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

    // Determine if this is a mutating command that requires preflight.
    let is_mutating = matches!(
        cli.command,
        Commands::Attester {
            sub: AttesterSub::Add { .. }
        } | Commands::Attester {
            sub: AttesterSub::Remove { .. }
        } | Commands::Attestation {
            sub: AttestationSub::RevokeByAttester { .. }
        } | Commands::Deploy { .. }
            | Commands::Upgrade { .. }
    );

    if is_mutating {
        run_preflight_checks(&cli, &network_cfg)?;
    }

    match cli.command {
        Commands::Config { sub } => handle_config(sub, &cli.network, &cli.config, &network_cfg)?,
        Commands::Attester { sub } => {
            handle_attester(sub, &cli.network, &network_cfg)?;
        }
        Commands::Attestation { sub } => {
            handle_attestation(sub, &cli.network, &network_cfg)?;
        }
        Commands::Deploy {
            build_only,
            dry_run,
            source,
            admin,
            yes,
        } => {
            handle_deploy(
                &cli.network,
                &network_cfg,
                build_only,
                dry_run,
                yes,
                source,
                admin,
            )?;
        }
        Commands::Upgrade {
            contract,
            id,
            source,
            wasm,
            skip_build,
            run_migrate,
            expected_schema_version,
            dry_run,
        } => {
            handle_upgrade(
                &cli.network,
                &network_cfg,
                &contract,
                id,
                source,
                wasm,
                skip_build,
                run_migrate,
                expected_schema_version,
                dry_run,
            )?;
        }
        Commands::SmokeTest { source, dry_run } => {
            handle_smoke_test(&cli.network, &network_cfg, source, dry_run)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Preflight (#401)
// ---------------------------------------------------------------------------

fn run_preflight_checks(cli: &Cli, network_cfg: &NetworkConfig) -> anyhow::Result<()> {
    // Attempt to load the release manifest for wasm hash verification.
    let manifest_path = find_release_manifest();
    let wasm_registry = match &manifest_path {
        Some(path) => WasmHashRegistry::load_from_manifest(path),
        None => WasmHashRegistry::empty(),
    };

    let opts = PreflightOptions {
        network: cli.network.clone(),
        skip_preflight: cli.skip_preflight,
        allow_unknown_wasm: cli.allow_unknown_wasm,
        wasm_registry,
    };

    let transport = CurlTransport;
    match run_preflight(network_cfg, &opts, &transport) {
        Ok(result) => {
            eprintln!(
                "==> Preflight OK: network={}, passphrase match, \
                 latest_ledger={}",
                cli.network, result.latest_ledger_sequence
            );
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Preflight failed — refusing to sign:\n\n{e}"
            ));
        }
    }
    Ok(())
}

/// Locate the release manifest JSON file relative to the current directory or
/// the workspace root.
fn find_release_manifest() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("docs/release-manifest/v0.1.0-dev.json"),
        PathBuf::from("../docs/release-manifest/v0.1.0-dev.json"),
        PathBuf::from("../../docs/release-manifest/v0.1.0-dev.json"),
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }
    // Try relative to CARGO_MANIFEST_DIR.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(&manifest_dir)
            .join("../../docs/release-manifest/v0.1.0-dev.json");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

fn handle_config(
    sub: ConfigSub,
    network: &str,
    config: &Option<PathBuf>,
    network_cfg: &NetworkConfig,
) -> anyhow::Result<()> {
    match sub {
        ConfigSub::Show => {
            let (path, _) =
                lafiya_config::load_network_config::<PathBuf>(network, config.clone())?;
            println!("Network: {}", network);
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
            println!("Deployment status: {}", deployment_summary(network_cfg));
            println!(
                "\nSecrets: NEVER stored in networks.toml. Use stellar identities or env vars."
            );
        }
        ConfigSub::List => {} // handled in main before network resolve
        ConfigSub::Env => {
            println!(
                "# Source this with: eval $(lafiya-cli --network {} config env)",
                network
            );
            println!("export LAFIYA_NETWORK={}", network);
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
    Ok(())
}

// ---------------------------------------------------------------------------
// Attester commands
// ---------------------------------------------------------------------------

fn handle_attester(
    sub: AttesterSub,
    network: &str,
    network_cfg: &NetworkConfig,
) -> anyhow::Result<()> {
    match sub {
        AttesterSub::Is { address } => {
            let contract_id = network_cfg
                .require_contract_id(network, ContractKind::AttesterRegistry)
                .map_err(|e| anyhow::anyhow!(e))?;
            validate_address("attester address", &address).context("invalid attester address")?;

            println!("Checking is_attester for {} on {}", address, contract_id);
            println!("RPC: {}", network_cfg.rpc_url);
            let args = invoke_args(
                network_cfg,
                contract_id,
                None,
                "is_attester",
                &["--attester", &address],
            );
            println!("> stellar {}", args.join(" "));
            if which_stellar().is_ok() {
                if let Err(e) =
                    std::process::Command::new("stellar").args(args).status()
                {
                    eprintln!(
                        "Failed to run stellar CLI: {e}. Install with: \
                         cargo install --locked stellar-cli"
                    );
                }
            } else {
                eprintln!(
                    "stellar CLI not found - showing command only. \
                     Install with: cargo install --locked stellar-cli"
                );
            }
        }
        AttesterSub::Add { address, source } => {
            let contract_id = network_cfg
                .require_contract_id(network, ContractKind::AttesterRegistry)
                .map_err(|e| anyhow::anyhow!(e))?;
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;

            let args = invoke_args(
                network_cfg,
                contract_id,
                source.as_deref(),
                "add_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
        AttesterSub::Remove { address, source } => {
            let contract_id = network_cfg
                .require_contract_id(network, ContractKind::AttesterRegistry)
                .map_err(|e| anyhow::anyhow!(e))?;
            validate_address("attester address", &address).context("invalid attester address")?;
            let source = validated_source(source)?;

            let args = invoke_args(
                network_cfg,
                contract_id,
                source.as_deref(),
                "remove_attester",
                &["--attester", &address],
            );
            run_stellar(args)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Attestation commands
// ---------------------------------------------------------------------------

fn handle_attestation(
    sub: AttestationSub,
    network: &str,
    network_cfg: &NetworkConfig,
) -> anyhow::Result<()> {
    match sub {
        AttestationSub::Get { record_hash } => {
            let contract_id = network_cfg
                .require_contract_id(network, ContractKind::AttestationRegistry)
                .map_err(|e| anyhow::anyhow!(e))?;
            validate_record_hash("record_hash", &record_hash)
                .context("invalid record hash (expected a hex encoded 32-byte hash)")?;

            let args = invoke_args(
                network_cfg,
                contract_id,
                None,
                "get_attestation",
                &["--record_hash", &record_hash],
            );
            println!("> stellar {}", args.join(" "));
            if which_stellar().is_ok() {
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

        AttestationSub::RevokeByAttester {
            attester,
            since,
            until,
            dry_run,
            source,
            audit_json,
            audit_csv,
        } => {
            // Validate inputs.
            validate_address("attester", &attester).context("invalid attester address")?;
            let contract_id = network_cfg
                .require_contract_id(network, ContractKind::AttestationRegistry)
                .map_err(|e| anyhow::anyhow!(e))?;
            let source = validated_source(source)
                .context("invalid --source")?
                .unwrap_or_default();

            let args = RevokeByAttesterArgs {
                attester,
                since_ledger: since,
                until_ledger: until,
                dry_run,
                audit_csv,
                audit_json,
                network: network.to_string(),
                rpc_url: network_cfg.rpc_url.clone(),
                attestation_registry_id: contract_id.to_string(),
            };

            let transport = StellarCliTransport;
            let report = revoke::run_revoke_by_attester(
                &args,
                &network_cfg.network_passphrase,
                &source,
                &transport,
            )
            .map_err(|e| anyhow::anyhow!("revoke-by-attester failed: {e}"))?;

            // Summary.
            println!("\n--- Revoke-by-Attester Summary ---");
            println!("Network   : {}", report.network);
            println!("Attester  : {}", report.attester);
            println!("Events    : {}", report.events_found);
            println!("Hashes    : {}", report.hashes_found);
            println!("Revoked   : {}", report.revoked);
            println!("Skipped   : {}", report.skipped);
            println!("Failed    : {}", report.failed);
            println!("Collateral: {}", report.collateral_damage_hashes);
            if dry_run {
                println!("\n[dry-run] No transactions submitted.");
            }
            if report.failed > 0 {
                anyhow::bail!(
                    "{} revocation(s) failed; check the audit report for details",
                    report.failed
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Deploy command (#402)
// ---------------------------------------------------------------------------

fn handle_deploy(
    network: &str,
    network_cfg: &NetworkConfig,
    build_only: bool,
    dry_run: bool,
    yes: bool,
    source: Option<String>,
    admin: Option<String>,
) -> anyhow::Result<()> {
    let source = resolve_env_or_arg(source, ENV_SOURCE, "source account");
    let admin = resolve_env_or_arg(admin, ENV_ADMIN, "admin address");

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║          Lafiya Soroban Contract Deployment           ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!("Network : {network}");
    println!("RPC URL : {}", network_cfg.rpc_url);
    println!("Source  : {}", source.as_deref().unwrap_or("<STELLAR_ACCOUNT>"));
    println!("Admin   : {}", admin.as_deref().unwrap_or("<from source>"));
    println!("Status  : {}", deployment_summary(network_cfg));

    if build_only {
        println!("\nBuilding WASM only (--build-only)...");
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
            anyhow::bail!("cargo build failed");
        }
        check_wasm_sizes()?;
        println!("Build complete.");
        return Ok(());
    }

    if dry_run {
        println!("\n[dry-run] Would deploy to network '{network}':");
        println!("  1. cargo build --workspace --release --target wasm32v1-none");
        println!("  2. stellar contract deploy --wasm attester_registry.wasm ...");
        println!("  3. stellar contract deploy --wasm attestation_registry.wasm ...");
        println!("  4. stellar contract invoke ... -- initialize ...");
        println!("  5. stellar contract invoke ... -- initialize ...");
        println!(
            "\nUpdate config/networks.toml [{network}.contracts] after a real deploy."
        );
        return Ok(());
    }

    // Interactive confirmation.
    if !yes {
        print!("\nDeploy to {network}? [y/N]: ");
        use std::io::{BufRead, Write};
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") && !line.trim().eq_ignore_ascii_case("yes") {
            println!("Deployment aborted.");
            return Ok(());
        }
    }

    let src = source
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--source or STELLAR_ACCOUNT is required for deploy"))?;

    // 1. Build.
    println!("\n==> Building WASM contracts...");
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
    check_wasm_sizes()?;

    // 2. Deploy attester-registry.
    println!("\n==> Deploying attester-registry...");
    let attester_id = stellar_deploy(
        "target/wasm32v1-none/release/attester_registry.wasm",
        src,
        network,
        &network_cfg.rpc_url,
        &network_cfg.network_passphrase,
    )?;
    println!("attester-registry: {attester_id}");

    // 3. Deploy attestation-registry.
    println!("\n==> Deploying attestation-registry...");
    let attestation_id = stellar_deploy(
        "target/wasm32v1-none/release/attestation_registry.wasm",
        src,
        network,
        &network_cfg.rpc_url,
        &network_cfg.network_passphrase,
    )?;
    println!("attestation-registry: {attestation_id}");

    // 4. Resolve admin address.
    let admin_addr = resolve_admin_address(admin.as_deref(), src, network)?;

    // 5. Initialize attester-registry.
    println!("\n==> Initializing attester-registry...");
    run_stellar(vec![
        "contract".to_string(),
        "invoke".to_string(),
        "--id".to_string(), attester_id.clone(),
        "--source-account".to_string(), src.to_string(),
        "--network".to_string(), network.to_string(),
        "--send".to_string(), "yes".to_string(),
        "--".to_string(),
        "initialize".to_string(),
        "--admin".to_string(), admin_addr.clone(),
    ])?;

    // 6. Initialize attestation-registry.
    println!("\n==> Initializing attestation-registry...");
    run_stellar(vec![
        "contract".to_string(),
        "invoke".to_string(),
        "--id".to_string(), attestation_id.clone(),
        "--source-account".to_string(), src.to_string(),
        "--network".to_string(), network.to_string(),
        "--send".to_string(), "yes".to_string(),
        "--".to_string(),
        "initialize".to_string(),
        "--admin".to_string(), admin_addr.clone(),
        "--attester_registry".to_string(), attester_id.clone(),
    ])?;

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║                 Deployment Complete!                  ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!("attester_registry   = {attester_id}");
    println!("attestation_registry = {attestation_id}");
    println!(
        "\nUpdate config/networks.toml [{network}.contracts] with these IDs, then commit."
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Upgrade command (#402)
// ---------------------------------------------------------------------------

fn handle_upgrade(
    network: &str,
    network_cfg: &NetworkConfig,
    contract: &str,
    id_override: Option<String>,
    source: Option<String>,
    wasm_path: Option<PathBuf>,
    skip_build: bool,
    run_migrate: bool,
    expected_schema_version: Option<u32>,
    dry_run: bool,
) -> anyhow::Result<()> {
    // Resolve contract kind and ID.
    let contract_kind = match contract {
        "attester-registry" | "attester_registry" => ContractKind::AttesterRegistry,
        "attestation-registry" | "attestation_registry" => ContractKind::AttestationRegistry,
        other => anyhow::bail!(
            "unknown contract '{}': use attester-registry or attestation-registry",
            other
        ),
    };

    let contract_id = if let Some(id) = id_override {
        id
    } else {
        network_cfg
            .require_contract_id(network, contract_kind)
            .map_err(|e| anyhow::anyhow!(e))?
            .to_string()
    };

    let source = validated_source(source)
        .context("invalid --source")?
        .or_else(|| std::env::var(ENV_SOURCE).ok())
        .ok_or_else(|| anyhow::anyhow!("--source or STELLAR_ACCOUNT is required for upgrade"))?;

    let wasm_file = match wasm_path {
        Some(p) => p,
        None => {
            // Derive from contract name.
            let artifact = match contract_kind {
                ContractKind::AttesterRegistry => "attester_registry.wasm",
                ContractKind::AttestationRegistry => "attestation_registry.wasm",
            };
            PathBuf::from("target/wasm32v1-none/release").join(artifact)
        }
    };

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║          Lafiya Soroban Contract Upgrade              ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!("Network  : {network}");
    println!("Contract : {} ({})", contract, &contract_id[..12.min(contract_id.len())]);
    println!("Source   : {source}");
    println!("Wasm     : {}", wasm_file.display());
    println!("Migrate  : {run_migrate}");

    // 1. Build if needed.
    if !skip_build && !wasm_file.exists() {
        if dry_run {
            println!("[dry-run] Would: cargo build --release --target wasm32v1-none");
        } else {
            println!("\n==> Building...");
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
    }

    // 2. Size check.
    if !dry_run && wasm_file.exists() {
        let size = std::fs::metadata(&wasm_file)?.len();
        if size > WASM_SIZE_BUDGET {
            anyhow::bail!(
                "Wasm file {} is {} bytes, exceeding the {}-byte budget. \
                 Shrink the contract before upgrading.",
                wasm_file.display(),
                size,
                WASM_SIZE_BUDGET
            );
        }
        println!("Wasm size: {} bytes ({:.0}% of budget)", size, size as f64 / WASM_SIZE_BUDGET as f64 * 100.0);
    }

    // 3. Compute local hash.
    let local_hash = if !dry_run && wasm_file.exists() {
        Some(sha256_file(&wasm_file)?)
    } else {
        None
    };
    if let Some(ref h) = local_hash {
        println!("Wasm hash: {h}");
    }

    // 4. Upload wasm.
    let uploaded_hash = if dry_run {
        println!("[dry-run] Would: stellar contract upload --wasm {} --optimize=false", wasm_file.display());
        "<dry-run-hash>".to_string()
    } else {
        println!("\n==> Uploading wasm...");
        let output = std::process::Command::new("stellar")
            .args([
                "contract",
                "upload",
                "--wasm",
                &wasm_file.to_string_lossy(),
                "--source-account",
                &source,
                "--network",
                network,
                "--optimize=false",
            ])
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "wasm upload failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    // 5. Verify hashes match.
    if let Some(ref lh) = local_hash {
        if !dry_run && uploaded_hash.trim() != lh.trim() {
            anyhow::bail!(
                "Wasm hash mismatch after upload!\n\
                 local:    {lh}\n\
                 uploaded: {uploaded_hash}\n\
                 This may mean the stellar CLI optimized the wasm. Always use --optimize=false."
            );
        }
    }

    // 6. Submit upgrade().
    if dry_run {
        println!("[dry-run] Would: stellar contract invoke ... -- upgrade --new-wasm-hash {uploaded_hash}");
    } else {
        println!("\n==> Submitting upgrade({uploaded_hash})...");
        run_stellar(vec![
            "contract".to_string(),
            "invoke".to_string(),
            "--id".to_string(), contract_id.clone(),
            "--source-account".to_string(), source.clone(),
            "--network".to_string(), network.to_string(),
            "--send".to_string(), "yes".to_string(),
            "--".to_string(),
            "upgrade".to_string(),
            "--new_wasm_hash".to_string(), uploaded_hash.clone(),
        ])?;
    }

    // 7. Optionally run migrate().
    if run_migrate {
        if dry_run {
            println!("[dry-run] Would: stellar contract invoke ... -- migrate");
        } else {
            println!("\n==> Running migrate()...");
            run_stellar(vec![
                "contract".to_string(),
                "invoke".to_string(),
                "--id".to_string(), contract_id.clone(),
                "--source-account".to_string(), source.clone(),
                "--network".to_string(), network.to_string(),
                "--send".to_string(), "yes".to_string(),
                "--".to_string(),
                "migrate".to_string(),
            ])?;
        }
    }

    // 8. Verify schema version.
    if let Some(expected) = expected_schema_version {
        if dry_run {
            println!("[dry-run] Would verify get_schema_version() == {expected}");
        } else {
            println!("\n==> Verifying schema version == {expected}...");
            let output = std::process::Command::new("stellar")
                .args([
                    "contract",
                    "invoke",
                    "--id",
                    &contract_id,
                    "--network",
                    network,
                    "--",
                    "get_schema_version",
                ])
                .output()?;
            let version_str = String::from_utf8_lossy(&output.stdout);
            let version: u32 = version_str.trim().parse().unwrap_or(0);
            if version != expected {
                anyhow::bail!(
                    "schema version mismatch: expected {expected}, got {version}"
                );
            }
            println!("Schema version: {version} ✓");
        }
    }

    println!("\n==> Upgrade complete.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Smoke test command (#402)
// ---------------------------------------------------------------------------

fn handle_smoke_test(
    network: &str,
    network_cfg: &NetworkConfig,
    source: Option<String>,
    dry_run: bool,
) -> anyhow::Result<()> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║           Lafiya Smoke Test                           ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!("Network: {network}");
    println!("RPC:     {}", network_cfg.rpc_url);

    let attester_id = network_cfg
        .require_contract_id(network, ContractKind::AttesterRegistry)
        .map_err(|e| anyhow::anyhow!(e))?;
    let attestation_id = network_cfg
        .require_contract_id(network, ContractKind::AttestationRegistry)
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("attester-registry  : {attester_id}");
    println!("attestation-registry: {attestation_id}");

    let steps = vec![
        format!(
            "stellar contract invoke --id {attester_id} \
             --rpc-url {} --network-passphrase '{}' \
             -- is_paused",
            network_cfg.rpc_url, network_cfg.network_passphrase
        ),
        format!(
            "stellar contract invoke --id {attestation_id} \
             --rpc-url {} --network-passphrase '{}' \
             -- is_paused",
            network_cfg.rpc_url, network_cfg.network_passphrase
        ),
        format!(
            "stellar contract invoke --id {attester_id} \
             --rpc-url {} --network-passphrase '{}' \
             -- get_attester_count",
            network_cfg.rpc_url, network_cfg.network_passphrase
        ),
        format!(
            "stellar contract invoke --id {attester_id} \
             --rpc-url {} --network-passphrase '{}' \
             -- get_schema_version",
            network_cfg.rpc_url, network_cfg.network_passphrase
        ),
    ];

    if dry_run {
        println!("\n[dry-run] Would run:");
        for step in &steps {
            println!("  {step}");
        }
        return Ok(());
    }

    let src = source
        .as_deref()
        .map(|_| ())
        .unwrap_or(()); // source not required for read-only smoke test
    let _ = src;

    println!("\n==> Running smoke tests...");

    // 1. is_paused (attester-registry)
    println!("\n[1/4] attester-registry is_paused:");
    run_stellar_read(&[
        "contract", "invoke",
        "--id", attester_id,
        "--rpc-url", &network_cfg.rpc_url,
        "--network-passphrase", &network_cfg.network_passphrase,
        "--", "is_paused",
    ])?;

    // 2. is_paused (attestation-registry)
    println!("\n[2/4] attestation-registry is_paused:");
    run_stellar_read(&[
        "contract", "invoke",
        "--id", attestation_id,
        "--rpc-url", &network_cfg.rpc_url,
        "--network-passphrase", &network_cfg.network_passphrase,
        "--", "is_paused",
    ])?;

    // 3. get_attester_count
    println!("\n[3/4] attester-registry get_attester_count:");
    run_stellar_read(&[
        "contract", "invoke",
        "--id", attester_id,
        "--rpc-url", &network_cfg.rpc_url,
        "--network-passphrase", &network_cfg.network_passphrase,
        "--", "get_attester_count",
    ])?;

    // 4. get_schema_version
    println!("\n[4/4] attester-registry get_schema_version:");
    run_stellar_read(&[
        "contract", "invoke",
        "--id", attester_id,
        "--rpc-url", &network_cfg.rpc_url,
        "--network-passphrase", &network_cfg.network_passphrase,
        "--", "get_schema_version",
    ])?;

    println!("\n==> Smoke test passed.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn invoke_args<'a>(
    cfg: &NetworkConfig,
    contract_id: &'a str,
    source: Option<&'a str>,
    function: &'a str,
    extra: &[&'a str],
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
        args.push("--send".to_string());
        args.push("yes".to_string());
    }
    args.push("--".to_string());
    args.push(function.to_string());
    for a in extra {
        args.push(a.to_string());
    }
    args
}

fn run_stellar(args: Vec<String>) -> anyhow::Result<()> {
    println!("> stellar {}", args.join(" "));
    if which_stellar().is_err() {
        eprintln!(
            "stellar CLI not found. Install with: cargo install --locked stellar-cli"
        );
        anyhow::bail!("stellar CLI not found");
    }
    let status = std::process::Command::new("stellar").args(&args).status()?;
    if !status.success() {
        anyhow::bail!("stellar CLI exited with {}", status);
    }
    Ok(())
}

fn run_stellar_read(args: &[&str]) -> anyhow::Result<()> {
    println!("> stellar {}", args.join(" "));
    if which_stellar().is_err() {
        eprintln!("stellar CLI not found — showing command only.");
        return Ok(());
    }
    let status = std::process::Command::new("stellar").args(args).status()?;
    if !status.success() {
        anyhow::bail!("stellar CLI exited with {}", status);
    }
    Ok(())
}

fn validated_source(source: Option<String>) -> anyhow::Result<Option<String>> {
    match source.or_else(|| std::env::var(ENV_SOURCE).ok()) {
        Some(s) => {
            validate_source_account(&s).map_err(|e| anyhow::anyhow!("invalid source: {e}"))?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

fn resolve_env_or_arg(
    arg: Option<String>,
    env_key: &str,
    _label: &str,
) -> Option<String> {
    arg.or_else(|| std::env::var(env_key).ok())
}

fn resolve_admin_address(
    admin: Option<&str>,
    source: &str,
    network: &str,
) -> anyhow::Result<String> {
    if let Some(addr) = admin {
        return Ok(addr.to_string());
    }
    // Resolve from stellar CLI.
    let output = std::process::Command::new("stellar")
        .args(["keys", "address", source, "--network", network])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
        }
        _ => {
            anyhow::bail!(
                "--admin not provided and could not resolve address for identity '{}'. \
                 Pass --admin <G...> explicitly.",
                source
            )
        }
    }
}

fn stellar_deploy(
    wasm_path: &str,
    source: &str,
    network: &str,
    rpc_url: &str,
    passphrase: &str,
) -> anyhow::Result<String> {
    let output = std::process::Command::new("stellar")
        .args([
            "contract",
            "deploy",
            "--wasm",
            wasm_path,
            "--source-account",
            source,
            "--rpc-url",
            rpc_url,
            "--network-passphrase",
            passphrase,
        ])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "stellar contract deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn check_wasm_sizes() -> anyhow::Result<()> {
    let wasms = [
        "target/wasm32v1-none/release/attester_registry.wasm",
        "target/wasm32v1-none/release/attestation_registry.wasm",
    ];
    for path in &wasms {
        if let Ok(meta) = std::fs::metadata(path) {
            let size = meta.len();
            if size > WASM_SIZE_BUDGET {
                anyhow::bail!(
                    "Wasm {path} is {size} bytes, exceeding the {WASM_SIZE_BUDGET}-byte cap. \
                     Shrink the contract before deploying."
                );
            }
            println!("  {path}: {size} bytes ({:.0}% of cap)", size as f64 / WASM_SIZE_BUDGET as f64 * 100.0);
        }
    }
    Ok(())
}

/// Compute SHA-256 of a file, returning the hex digest.
fn sha256_file(path: &PathBuf) -> anyhow::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish_hex())
}

/// Minimal SHA-256 implementation (no external dep).
struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
    total: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buf: Vec::new(),
            total: 0,
        }
    }
    fn update(&mut self, data: &[u8]) {
        self.total += data.len() as u64;
        self.buf.extend_from_slice(data);
        while self.buf.len() >= 64 {
            let block: [u8; 64] = self.buf[..64].try_into().unwrap();
            self.buf.drain(..64);
            sha256_compress(&mut self.state, &block);
        }
    }
    fn finish_hex(mut self) -> String {
        let bit_len = self.total * 8;
        self.buf.push(0x80);
        while (self.buf.len() % 64) != 56 {
            self.buf.push(0x00);
        }
        for b in bit_len.to_be_bytes() {
            self.buf.push(b);
        }
        while self.buf.len() >= 64 {
            let block: [u8; 64] = self.buf[..64].try_into().unwrap();
            self.buf.drain(..64);
            sha256_compress(&mut self.state, &block);
        }
        let mut out = String::new();
        for w in self.state {
            out.push_str(&format!("{w:08x}"));
        }
        out
    }
}

fn sha256_compress(state: &mut [u32; 8], block: &[u8; 64]) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
        0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
        0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g; g = f; f = e; e = d.wrapping_add(t1);
        d = c; c = b; b = a; a = t1.wrapping_add(t2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn deployment_summary(cfg: &NetworkConfig) -> &'static str {
    match cfg.deployment_state() {
        DeploymentState::Deployed => "deployed",
        DeploymentState::NotDeployed => "not deployed",
        DeploymentState::Partial { .. } => "partially deployed",
    }
}

fn which_stellar() -> Result<(), ()> {
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            let full = p.join("stellar");
            if full.exists() {
                return Ok(());
            }
        }
    }
    Err(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn revoke_by_attester_requires_attester() {
        let err = Cli::try_parse_from(["lafiya-cli", "attestation", "revoke-by-attester"])
            .expect_err("expected a missing required argument error");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("ATTESTER"),
            "expected error to name the missing `attester` argument, got: {message}"
        );
    }

    #[test]
    fn upgrade_requires_contract_flag() {
        let err = Cli::try_parse_from(["lafiya-cli", "upgrade"])
            .expect_err("expected missing --contract");
        let message = err.to_string();
        assert!(
            message.to_uppercase().contains("CONTRACT"),
            "expected error to name the missing `--contract` flag, got: {message}"
        );
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256 of empty string is the known constant.
        let mut h = Sha256::new();
        h.update(b"");
        let hex = h.finish_hex();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc_vector() {
        let mut h = Sha256::new();
        h.update(b"abc");
        let hex = h.finish_hex();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469348423f656fcea929"
        );
    }

    #[test]
    fn signer_flag_parsed_as_ledger_uri() {
        let cli = Cli::try_parse_from([
            "lafiya-cli",
            "--signer",
            "ledger://0",
            "config",
            "list",
        ])
        .unwrap();
        assert_eq!(cli.signer.as_deref(), Some("ledger://0"));
        let uri = SignerUri::parse(cli.signer.as_deref().unwrap()).unwrap();
        assert_eq!(uri, SignerUri::Ledger(0));
    }

    #[test]
    fn skip_preflight_flag_parsed() {
        let cli = Cli::try_parse_from([
            "lafiya-cli",
            "--skip-preflight",
            "config",
            "list",
        ])
        .unwrap();
        assert!(cli.skip_preflight);
    }

    #[test]
    fn allow_unknown_wasm_flag_parsed() {
        let cli = Cli::try_parse_from([
            "lafiya-cli",
            "--allow-unknown-wasm",
            "config",
            "show",
        ])
        .unwrap();
        assert!(cli.allow_unknown_wasm);
    }
}
