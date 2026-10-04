use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use lafiya_indexer::api::{self, AppState};
use lafiya_indexer::decode::RawEvent;
use lafiya_indexer::ingest::{self, Ingestor};
use lafiya_indexer::metrics::Metrics;
use lafiya_indexer::reconcile::Reconciler;
use lafiya_indexer::rpc::RpcClient;
use lafiya_indexer::store::MIGRATOR;
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(version, about = "Lafiya contract event indexer")]
struct Cli {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply database migrations and exit.
    Migrate,
    /// Ingest events, reconcile, and serve the API.
    Run(Box<RunArgs>),
    /// Load archived events (NDJSON of `getEvents` event objects) older than
    /// anything ingested so far.
    Backfill {
        #[arg(long)]
        file: PathBuf,
        /// The export is complete through this ledger; live ingestion resumes
        /// at the next one.
        #[arg(long)]
        through_ledger: Option<u32>,
    },
}

#[derive(clap::Args)]
struct RunArgs {
    #[arg(long, env = "LAFIYA_RPC_URL")]
    rpc_url: String,
    #[arg(long, env = "LAFIYA_ATTESTER_REGISTRY")]
    attester_registry: String,
    #[arg(long, env = "LAFIYA_ATTESTATION_REGISTRY")]
    attestation_registry: String,
    /// First ledger to ingest when the database has no checkpoint.
    #[arg(long, env = "LAFIYA_START_LEDGER")]
    start_ledger: Option<u32>,
    #[arg(long, env = "LAFIYA_LISTEN", default_value = "0.0.0.0:8080")]
    listen: SocketAddr,
    #[arg(long, env = "LAFIYA_POLL_INTERVAL_SECS", default_value_t = 5)]
    poll_interval_secs: u64,
    #[arg(long, env = "LAFIYA_PAGE_LIMIT", default_value_t = 200)]
    page_limit: u32,
    #[arg(long, env = "LAFIYA_RECONCILE_INTERVAL_SECS", default_value_t = 300)]
    reconcile_interval_secs: u64,
    #[arg(long, env = "LAFIYA_RECONCILE_SAMPLE", default_value_t = 50)]
    reconcile_sample: i64,
    #[arg(long, env = "LAFIYA_RECONCILE_MAX_LAG", default_value_t = 2)]
    reconcile_max_lag: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&cli.database_url)
        .await
        .context("connecting to Postgres")?;
    MIGRATOR.run(&pool).await.context("running migrations")?;

    match cli.command {
        Command::Migrate => Ok(()),
        Command::Backfill {
            file,
            through_ledger,
        } => {
            let reader = std::io::BufReader::new(
                std::fs::File::open(&file)
                    .with_context(|| format!("opening {}", file.display()))?,
            );
            let mut events = Vec::new();
            for (n, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let event: RawEvent = serde_json::from_str(&line)
                    .with_context(|| format!("{}:{}", file.display(), n + 1))?;
                events.push(event);
            }
            let applied = ingest::backfill(&pool, events, through_ledger).await?;
            tracing::info!(applied, "backfill complete");
            Ok(())
        }
        Command::Run(args) => run(pool, *args).await,
    }
}

async fn run(pool: sqlx::PgPool, args: RunArgs) -> anyhow::Result<()> {
    let metrics = Arc::new(Metrics::new());
    let rpc = RpcClient::new(
        &args.rpc_url,
        vec![
            args.attester_registry.clone(),
            args.attestation_registry.clone(),
        ],
    );
    let ingestor = Ingestor {
        pool: pool.clone(),
        source: rpc.clone(),
        metrics: metrics.clone(),
        start_ledger: args.start_ledger,
        page_limit: args.page_limit,
    };
    let reconciler = Reconciler {
        pool: pool.clone(),
        rpc,
        metrics: metrics.clone(),
        attester_registry: args.attester_registry,
        attestation_registry: args.attestation_registry,
        sample_size: args.reconcile_sample,
        max_lag_ledgers: args.reconcile_max_lag,
    };
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "serving API");
    let app = api::router(AppState { pool, metrics });

    tokio::select! {
        err = ingestor.run(Duration::from_secs(args.poll_interval_secs)) => {
            tracing::error!(error = %err, "ingestion stopped");
            Err(err.into())
        }
        _ = reconciler.run(Duration::from_secs(args.reconcile_interval_secs)) => unreachable!(),
        served = axum::serve(listener, app) => served.context("API server"),
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
            Ok(())
        }
    }
}
