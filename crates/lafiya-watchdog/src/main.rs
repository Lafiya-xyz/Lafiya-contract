//! `lafiya-watchdog run` watches live; `lafiya-watchdog replay` evaluates the
//! same rules over a historical ledger range (or a JSONL event file) for
//! incident forensics.

use clap::{Parser, Subcommand};
use lafiya_watchdog::config::Config;
use lafiya_watchdog::rpc::Rpc;
use lafiya_watchdog::rules::{Engine, HealthMonitor};
use lafiya_watchdog::{sinks, Alert, Event};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

/// Routes alerts through the dedup/grouping/escalation pipeline before
/// emitting them. Returns the alerts that survived deduplication.
fn emit(cfg: &Config, engine: &mut Engine, alert: &Alert, send: bool) -> Vec<Alert> {
    let routed = engine.route_alert(alert);
    for a in &routed {
        tracing::warn!(kind = %a.kind, severity = %a.severity, "{}", a.summary);
        println!("{}", serde_json::to_string(a).expect("alert serializes"));
        if send {
            for err in sinks::dispatch(&cfg.sinks, a) {
                tracing::error!(error = %err, "alert sink failed");
            }
        }
    }
    routed
}

/// Persist engine state (rule windows, alert fingerprints, escalation state)
/// so a restart does not re-alert or lose in-flight windows.
fn persist(cfg: &Config, engine: &Engine) {
    if let Err(e) = engine.save_state(&cfg.state_path) {
        tracing::error!(error = %e, "failed to persist watchdog state");
    }
}

/// Spawn the dead-man's-switch heartbeat task. It pings the external monitor
/// after every healthy poll; absence of pings pages the operator.
fn spawn_heartbeat(cfg: &Config, stop: Arc<AtomicBool>) {
    let Some(url) = cfg.heartbeat.ping_url.clone() else {
        return;
    };
    let interval = Duration::from_secs(cfg.heartbeat.ping_interval_secs.max(1));
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            if let Err(e) = ureq::get(&url).call() {
                tracing::warn!(error = %e, "heartbeat ping failed");
            }
            std::thread::sleep(interval);
        }
    });
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
    let mut engine = Engine::load(&cfg, &cfg.state_path)?;
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
                    emit(&cfg, &mut engine, &alert, send);
                }
            }
            if send {
                persist(&cfg, &engine);
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
            // Resume from persisted cursor so a kill -9 restart neither
            // re-alerts nor loses in-flight windows.
            let mut next = match start_ledger {
                Some(l) => l,
                None => engine
                    .resume_ledger()
                    .unwrap_or(rpc.latest_ledger()?),
            };
            let mut health = HealthMonitor::default();
            let stop = Arc::new(AtomicBool::new(false));
            spawn_heartbeat(&cfg, stop.clone());
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
                            emit(&cfg, &mut engine, &a, true);
                        }
                        next = processed + 1;
                        engine.set_resume_ledger(next);
                        persist(&cfg, &engine);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "poll failed");
                        if let Some(a) = health
                            .record_failure(cfg.heartbeat.max_consecutive_failures, &e.to_string())
                        {
                            emit(&cfg, &mut engine, &a, true);
                        }
                        // Persist even on failure so partial progress and
                        // alert fingerprints survive a crash.
                        persist(&cfg, &engine);
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
            // Critical rules attach a pre-built unsigned pause proposal;
            // guardian signatures are required via multisig tooling.
            let alert = engine.attach_pause_proposal(alert);
            emit(cfg, engine, &alert, true);
        }
    }
    for contract in &cfg.contracts {
        let hash = rpc.contract_wasm_hash(contract)?;
        if let Some(alert) = engine.observe_wasm(contract, &hash, latest) {
            let alert = engine.attach_pause_proposal(alert);
            emit(cfg, engine, &alert, true);
        }
    }
    Ok(latest)
}
