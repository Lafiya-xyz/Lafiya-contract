//! `lafiya-watchdog run` watches live; `lafiya-watchdog replay` evaluates the
//! same rules over a historical ledger range (or a JSONL event file) for
//! incident forensics.

use clap::{Parser, Subcommand};
use lafiya_watchdog::config::Config;
use lafiya_watchdog::rpc::Rpc;
use lafiya_watchdog::rules::{Engine, HealthMonitor};
use lafiya_watchdog::{sinks, Alert, Event};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "lafiya-watchdog",
    about = "Security watchdog for privileged Lafiya contract events"
)]
struct Cli {
    /// Path to watchdog.toml
    #[arg(long, default_value = "watchdog.toml", global = true)]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Follow the chain and alert in real time
    Run {
        /// Ledger to start from (default: latest)
        #[arg(long)]
        start_ledger: Option<u32>,
    },
    /// Evaluate rules over past events; alerts are printed, not sent,
    /// unless --send is given
    Replay {
        #[arg(long, required_unless_present = "events")]
        from: Option<u32>,
        #[arg(long)]
        to: Option<u32>,
        /// Read events from a JSONL file instead of RPC
        #[arg(long)]
        events: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        send: bool,
    },
    /// Write the watched contracts' events in [from, to) as JSONL to stdout
    /// (input for replay and for the transparency dashboard)
    Export {
        #[arg(long)]
        from: u32,
        #[arg(long)]
        to: Option<u32>,
    },
}

fn emit(cfg: &Config, alert: &Alert, send: bool) {
    tracing::warn!(kind = %alert.kind, severity = %alert.severity, "{}", alert.summary);
    println!(
        "{}",
        serde_json::to_string(alert).expect("alert serializes")
    );
    if send {
        for err in sinks::dispatch(&cfg.sinks, alert) {
            tracing::error!(error = %err, "alert sink failed");
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LAFIYA_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    let mut engine = Engine::new(&cfg);
    let rpc = Rpc::new(&cfg.rpc_url);

    match cli.command {
        Command::Replay {
            from,
            to,
            events,
            send,
        } => {
            let events: Vec<Event> = match events {
                Some(path) => std::fs::read_to_string(path)?
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(serde_json::from_str)
                    .collect::<Result<_, _>>()?,
                None => rpc.events(&cfg.contracts, from.unwrap_or(0), to)?.0,
            };
            for e in &events {
                for alert in engine.evaluate(e) {
                    emit(&cfg, &alert, send);
                }
            }
            tracing::info!(events = events.len(), "replay finished");
        }
        Command::Export { from, to } => {
            let (events, latest) = rpc.events(&cfg.contracts, from, to)?;
            for e in &events {
                println!("{}", serde_json::to_string(e)?);
            }
            tracing::info!(events = events.len(), latest, "export finished");
        }
        Command::Run { start_ledger } => {
            let mut next = match start_ledger {
                Some(l) => l,
                None => rpc.latest_ledger()?,
            };
            let mut health = HealthMonitor::default();
            tracing::info!(
                ledger = next,
                contracts = cfg.contracts.len(),
                "watchdog started"
            );
            loop {
                match poll_once(&cfg, &rpc, &mut engine, next) {
                    Ok(latest) => {
                        health.record_success();
                        // Only advance past fully processed ledgers.
                        let processed = latest.max(next);
                        if let Some(a) =
                            health.check_lag(processed, latest, cfg.heartbeat.max_lag_ledgers)
                        {
                            emit(&cfg, &a, true);
                        }
                        next = processed + 1;
                        if let Some(url) = &cfg.heartbeat.ping_url {
                            if let Err(e) = ureq::get(url).call() {
                                tracing::warn!(error = %e, "heartbeat ping failed");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "poll failed");
                        if let Some(a) = health
                            .record_failure(cfg.heartbeat.max_consecutive_failures, &e.to_string())
                        {
                            emit(&cfg, &a, true);
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(cfg.poll_interval_secs));
            }
        }
    }
    Ok(())
}

/// Process events from `start` up to the latest ledger and run the wasm
/// consistency check. Returns the latest ledger seen.
fn poll_once(cfg: &Config, rpc: &Rpc, engine: &mut Engine, start: u32) -> anyhow::Result<u32> {
    let (events, latest) = rpc.events(&cfg.contracts, start, None)?;
    for e in &events {
        for alert in engine.evaluate(e) {
            emit(cfg, &alert, true);
        }
    }
    for contract in &cfg.contracts {
        let hash = rpc.contract_wasm_hash(contract)?;
        if let Some(alert) = engine.observe_wasm(contract, &hash, latest) {
            emit(cfg, &alert, true);
        }
    }
    Ok(latest)
}
