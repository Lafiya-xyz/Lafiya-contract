//! Reference event indexer for the Lafiya contracts.
//!
//! Ingests `getEvents` with cursor checkpointing into Postgres, reconciles
//! derived state against on-chain storage, and serves a read-only query API.
//! See `docs/indexer.md` for deployment.

pub mod api;
pub mod archive;
pub mod decode;
pub mod delivery;
pub mod ingest;
pub mod metrics;
pub mod reconcile;
pub mod reorg;
pub mod rpc;
pub mod store;

pub use ingest::{IngestConfig, IngestError, Ingestor};
pub use reorg::{FinalityInfo, ReorgDetector, ReorgError, RollbackOutcome};
pub use store::{IndexerStore, StoreError};
