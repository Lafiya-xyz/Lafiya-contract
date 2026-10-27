use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use lafiya_indexer::api::{self, AppState};
use lafiya_indexer::archive::{ArchiveSource, ArchiveVerifier};
use lafiya_indexer::decode::RawEvent;
use lafiya_indexer::ingest::{self, Ingestor};
use lafiya_indexer::leader::{LeaderElector, Role};
use lafiya_indexer::metrics::Metrics;
use lafiya_indexer::prune::{self, PruneConfig};
use lafiya_indexer::reconcile::Reconciler;
use lafiya_indexer::rpc::{EventSource, RpcClient};
use lafiya_indexer::store::MIGRATOR;
use lafiya_indexer::webhook::{self, WebhookDispatcher};
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
        /// Only ingest events in this inclusive ledger range. Backfill is
        /// resumable: re-running with the same range skips already-applied
        /// ledgers, and disjoint ranges may be run in parallel.
        #[arg(long)]
        from_ledger: Option<u32>,
        #[arg(long)]
        to_ledger: Option<u32>,
        /// Checkpoint hash(es) the archive must verify against. Repeatable.
        /// Without at least one, the backfill is refused.
        #[arg(long = "checkpoint", env = "LAFIYA_ARCHIVE_CHECKPOINTS")]
        checkpoints: Vec<String>,
        /// Skip checkpoint verification. Only for trusted local exports.
        #[arg(long, env = "LAFIYA_ARCHIVE_INSECURE")]
        insecure: bool,
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
    /// How often the webhook dispatcher scans for due deliveries.
    #[arg(long, env = "LAFIYA_WEBHOOK_INTERVAL_SECS", default_value_t = 5)]
    webhook_interval_secs: u64,
    /// HMAC secret used to sign webhook payloads. Required to enable delivery.
    #[arg(long, env = "LAFIYA_WEBHOOK_SECRET")]
    webhook_secret: Option<String>,
    /// Maximum delivery attempts before a webhook is dead-lettered.
    #[arg(long, env = "LAFIYA_WEBHOOK_MAX_ATTEMPTS", default_value_t = 8)]
    webhook_max_attempts: i32,
    /// How often the retention/pruning job runs.
    #[arg(long, env = "LAFIYA_PRUNE_INTERVAL_SECS", default_value_t = 3600)]
    prune_interval_secs: u64,
    /// Retain derived/operational rows for this many days.
    #[arg(long, env = "LAFIYA_RETENTION_DAYS", default_value_t = 30)]
    retention_days: i64,
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
            from_ledger,
            to_ledger,
            checkpoints,
            insecure,
        } => {
            if !insecure && checkpoints.is_empty() {
                anyhow::bail!(
                    "refusing unverifiable archive: pass --checkpoint <hash> (repeatable) \
                     or --insecure for a trusted local export"
                );
            }
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
            let source = ArchiveSource::new(events);
            let verifier = if insecure {
                ArchiveVerifier::insecure()
            } else {
                ArchiveVerifier::new(checkpoints)
            };
            let range = match (from_ledger, to_ledger) {
                (None, None) => None,
                (from, to) => Some((from.unwrap_or(0), to.unwrap_or(u32::MAX))),
            };
            let applied = ingest::backfill_verified(
                &pool,
                &source,
                &verifier,
                range,
                through_ledger,
            )
            .await?;
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
    let leader = LeaderElector::new(pool.clone(), args.leader_lock_key);
    let role = Arc::new(leader.role_handle());
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "serving API");
    let app = api::router(AppState {
        pool: pool.clone(),
        metrics: metrics.clone(),
        role: role.clone(),
    });

    // Followers serve reads only; they poll for leadership and take over
    // ingestion when the active leader's advisory lock is released.
    let follower_pool = pool.clone();
    let follower_metrics = metrics.clone();
    let follower_role = role.clone();
    let follower = async move {
        loop {
            if follower_role.get() == Role::Leader {
                tokio::time::sleep(Duration::from_secs(args.leader_retry_secs)).await;
                continue;
            }
            match leader.try_acquire().await {
                Ok(Some(guard)) => {
                    tracing::info!("acquired leadership");
                    follower_role.set(Role::Leader);
                    let ingestor = Ingestor {
                        pool: follower_pool.clone(),
                        source: rpc.clone(),
                        metrics: follower_metrics.clone(),
                        start_ledger: args.start_ledger,
                        page_limit: args.page_limit,
                    };
                    let reconciler = Reconciler {
                        pool: follower_pool.clone(),
                        rpc: rpc.clone(),
                        metrics: follower_metrics.clone(),
                        attester_registry: args.attester_registry.clone(),
                        attestation_registry: args.attestation_registry.clone(),
                        sample_size: args.reconcile_sample,
                        max_lag_ledgers: args.reconcile_max_lag,
                    };
                    tokio::select! {
                        err = ingestor.run(Duration::from_secs(args.poll_interval_secs)) => {
                            tracing::error!(error = %err, "ingestion stopped");
                            return Err(err.into());
                        }
                        _ = reconciler.run(Duration::from_secs(args.reconcile_interval_secs)) => unreachable!(),
                        _ = guard.lost() => {
                            tracing::warn!("lost leadership");
                            follower_role.set(Role::Follower);
                        }
                    }
                }
                Ok(None) => {
                    follower_role.set(Role::Follower);
                    tokio::time::sleep(Duration::from_secs(args.leader_retry_secs)).await;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "leadership check failed");
                    tokio::time::sleep(Duration::from_secs(args.leader_retry_secs)).await;
                }
            }
        }
    };
    let webhook_secret = match args.webhook_secret {
        Some(secret) if !secret.is_empty() => Some(Arc::new(secret.into_bytes())),
        Some(_) => anyhow::bail!("LAFIYA_WEBHOOK_SECRET must not be empty"),
        None => {
            tracing::warn!("no webhook secret configured; webhook delivery disabled");
            None
        }
    };
    let dispatcher = webhook_secret.map(|secret| WebhookDispatcher {
        pool: pool.clone(),
        secret,
        max_attempts: args.webhook_max_attempts,
    });
    let prune_config = PruneConfig {
        retention_days: args.retention_days,
    };

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "serving API");
    let app = api::router(AppState { pool, metrics });

    let webhook_task = async {
        match &dispatcher {
            Some(dispatcher) => {
                dispatcher
                    .run(Duration::from_secs(args.webhook_interval_secs))
                    .await
            }
            None => std::future::pending().await,
        }
    };
    let prune_task = prune::run(
        pool.clone(),
        prune_config,
        Duration::from_secs(args.prune_interval_secs),
    );

    tokio::select! {
        err = ingestor.run(Duration::from_secs(args.poll_interval_secs)) => {
            tracing::error!(error = %err, "ingestion stopped");
            Err(err.into())
        }
        _ = reconciler.run(Duration::from_secs(args.reconcile_interval_secs)) => unreachable!(),
        _ = webhook_task => {
            tracing::error!("webhook dispatcher stopped");
            Err(anyhow::anyhow!("webhook dispatcher stopped"))
        }
        _ = prune_task => {
            tracing::error!("pruning job stopped");
            Err(anyhow::anyhow!("pruning job stopped"))
        }
        served = axum::serve(listener, app) => served.context("API server"),
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
            Ok(())
        }
    }
}
