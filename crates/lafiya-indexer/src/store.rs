//! Postgres storage: applying event pages atomically with the checkpoint.

use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::decode::{cursor_ledger, decode, DecodeError, LafiyaEvent, RawEvent};

/// Default number of rows deleted per batch by [`prune`].
pub const PRUNE_BATCH_SIZE: i64 = 500;
/// Default maximum number of batches [`prune`] will run per table per call.
pub const PRUNE_MAX_BATCHES: i64 = 20;
/// Default retention window (in seconds) for operational tables.
pub const DEFAULT_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;
/// Maximum delivery attempts before a delivery is dead-lettered.
pub const MAX_DELIVERY_ATTEMPTS: i32 = 8;
/// Base backoff (seconds) for the first retry; doubles per attempt.
pub const BACKOFF_BASE_SECS: i64 = 5;
/// Cap on the exponential backoff (seconds).
pub const BACKOFF_MAX_SECS: i64 = 60 * 60;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const LIVE: &str = "live";

/// Advisory lock key identifying the single active ingester. Chosen as a
/// fixed, application-specific constant so replicas agree on it.
pub const LEADER_LOCK_KEY: i64 = 0x6c616669_7961_0001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Last RPC cursor, or `None` to resume from `ledger` via `startLedger`.
    pub cursor: Option<String>,
    pub ledger: u32,
    /// Hash of the ledger checkpoint covering `ledger`, when known. Used to
    /// verify archive data against the chain and to hand off archive -> live.
    pub checkpoint_hash: Option<String>,
    /// Monotonically increasing fencing token; stale writers are rejected.
    pub fencing_token: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The stored ledger hash for `ledger` differs from the one the provider
    /// served: the chain tip diverged and derived state must be rolled back.
    #[error("ledger {ledger} hash mismatch: stored {stored}, provider {observed}")]
    Reorg {
        ledger: u32,
        stored: String,
        observed: String,
    },
}

/// A ledger hash recorded at ingest time, used to verify chain continuity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHash {
    pub ledger: u32,
    pub hash: String,
}

/// A work item in the incentive pool.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct WorkItem {
    pub id: String,
    pub attester: String,
    pub amount: i64,
    pub status: String,
    pub created_ledger: i64,
    pub updated_ledger: i64,
}

/// A payout from the incentive pool.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Payout {
    pub id: String,
    pub work_item_id: String,
    pub attester: String,
    pub amount: i64,
    pub ledger: i64,
    pub tx_hash: String,
}

/// A push-delivery subscription. Filters are optional; `None` matches all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: i64,
    pub url: String,
    pub secret: String,
    pub contract_id: Option<String>,
    pub event_type: Option<String>,
    pub attester: Option<String>,
    pub active: bool,
}

/// A delivery claimed for sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: i64,
    pub subscription_id: i64,
    pub event_id: String,
    pub url: String,
    pub secret: String,
    pub attempts: i32,
}

/// Retention configuration for [`prune`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Delete deliveries/dead letters older than this many seconds.
    pub retention_secs: i64,
    /// Rows deleted per batch (bounded lock duration).
    pub batch_size: i64,
    /// Maximum batches per table per call.
    pub max_batches: i64,
}

impl Default for Retention {
    fn default() -> Self {
        Retention {
            retention_secs: DEFAULT_RETENTION_SECS,
            batch_size: PRUNE_BATCH_SIZE,
            max_batches: PRUNE_MAX_BATCHES,
        }
    }
}

/// Rows removed by a [`prune`] call, per table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    pub deliveries: u64,
    pub dead_letters: u64,
}

/// Exponential backoff in seconds for a given attempt count (1-based).
pub fn backoff_secs(attempts: i32) -> i64 {
    let shift = attempts.saturating_sub(1).clamp(0, 20) as u32;
    BACKOFF_BASE_SECS
        .saturating_mul(1i64 << shift)
        .min(BACKOFF_MAX_SECS)
}

pub async fn checkpoint(pool: &PgPool) -> Result<Option<Checkpoint>, sqlx::Error> {
    let row: Option<(Option<String>, i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT cursor, ledger, checkpoint_hash, fencing_token
         FROM checkpoints WHERE name = $1",
    )
    .bind(LIVE)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(cursor, ledger, checkpoint_hash, fencing_token)| Checkpoint {
        cursor,
        ledger: ledger as u32,
        checkpoint_hash,
        fencing_token,
    }))
}

/// Try to become the single active ingester. Returns `true` if this session
/// holds the leader advisory lock. The lock is session-scoped, so it is
/// released automatically if the connection drops (leader crash).
pub async fn try_acquire_leadership(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let (acquired,): (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(LEADER_LOCK_KEY)
        .fetch_one(pool)
        .await?;
    Ok(acquired)
}

/// Release the leader advisory lock, if held by this session.
pub async fn release_leadership(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LEADER_LOCK_KEY)
        .execute(pool)
        .await?;
    Ok(())
}

/// Bump the fencing token and return the new value. Called by the leader when
/// it takes over so that any in-flight writes from a previous leader are
/// rejected by `apply_page`.
pub async fn bump_fencing_token(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (token,): (i64,) = sqlx::query_as(
        "INSERT INTO checkpoints (name, cursor, ledger, fencing_token)
         VALUES ($1, NULL, 0, 1)
         ON CONFLICT (name) DO UPDATE
         SET fencing_token = checkpoints.fencing_token + 1, updated_at = now()
         RETURNING fencing_token",
    )
    .bind(LIVE)
    .fetch_one(pool)
    .await?;
    Ok(token)
}

/// Apply `events` (in chain order) and move the checkpoint to `next`, in one
/// transaction: a crash either keeps or discards the whole page, and events
/// already in `raw_events` are never applied twice. Returns the number of
/// newly applied events.
pub async fn apply_page(
    pool: &PgPool,
    events: &[RawEvent],
    next: &Checkpoint,
) -> Result<usize, StoreError> {
    let mut tx = pool.begin().await?;
    // Reject stale writers: the stored fencing token must not exceed ours.
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT fencing_token FROM checkpoints WHERE name = $1 FOR UPDATE")
            .bind(LIVE)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((current,)) = current {
        if next.fencing_token < current {
            return Err(StoreError::StaleWriter {
                attempted: next.fencing_token,
                current,
            });
        }
    }
    let mut applied = 0;
    for raw in events.iter().filter(|e| e.in_successful_contract_call) {
        let event = decode(raw)?;
        let inserted = sqlx::query(
            "INSERT INTO raw_events
                 (id, contract_id, ledger, ledger_closed_at, tx_hash, name, topics, value, decoded)
             VALUES ($1, $2, $3, $4::text::timestamptz, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&raw.id)
        .bind(&raw.contract_id)
        .bind(i64::from(raw.ledger))
        .bind(&raw.ledger_closed_at)
        .bind(&raw.tx_hash)
        .bind(event.name())
        .bind(&raw.topic)
        .bind(&raw.value)
        .bind(serde_json::to_value(&event).unwrap_or(Value::Null))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            apply_event(&mut tx, raw, &event).await?;
            enqueue_deliveries(&mut tx, raw, &event).await?;
            applied += 1;
        }
    }
    sqlx::query(
        "INSERT INTO checkpoints (name, cursor, ledger, checkpoint_hash, fencing_token)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (name) DO UPDATE
         SET cursor = EXCLUDED.cursor, ledger = EXCLUDED.ledger,
             checkpoint_hash = EXCLUDED.checkpoint_hash,
             fencing_token = EXCLUDED.fencing_token, updated_at = now()",
    )
    .bind(LIVE)
    .bind(&next.cursor)
    .bind(i64::from(next.ledger))
    .bind(&next.checkpoint_hash)
    .bind(next.fencing_token)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(applied)
}

/// Record the hash of an ingested ledger. Returns `Err(StoreError::Reorg)` if a
/// different hash was already stored for the same ledger, so the caller can
/// roll back to the common ancestor before re-ingesting.
pub async fn record_ledger_hash(
    pool: &PgPool,
    ledger: u32,
    hash: &str,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT hash FROM ledger_hashes WHERE ledger = $1")
            .bind(i64::from(ledger))
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(stored) = existing {
        if stored != hash {
            return Err(StoreError::Reorg {
                ledger,
                stored,
                observed: hash.to_string(),
            });
        }
    } else {
        sqlx::query("INSERT INTO ledger_hashes (ledger, hash) VALUES ($1, $2)")
            .bind(i64::from(ledger))
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Verify that every ledger in `hashes` matches what is stored, returning the
/// first mismatch (if any) as a `StoreError::Reorg`.
pub async fn verify_ledger_hashes(
    pool: &PgPool,
    hashes: &[LedgerHash],
) -> Result<(), StoreError> {
    for expected in hashes {
        let stored: Option<String> =
            sqlx::query_scalar("SELECT hash FROM ledger_hashes WHERE ledger = $1")
                .bind(i64::from(expected.ledger))
                .fetch_optional(pool)
                .await?;
        if let Some(stored) = stored {
            if stored != expected.hash {
                return Err(StoreError::Reorg {
                    ledger: expected.ledger,
                    stored,
                    observed: expected.hash.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Highest ledger whose hash is recorded, i.e. the last known-good tip.
pub async fn last_hashed_ledger(pool: &PgPool) -> Result<Option<u32>, sqlx::Error> {
    let ledger: Option<i64> = sqlx::query_scalar("SELECT MAX(ledger) FROM ledger_hashes")
        .fetch_one(pool)
        .await?;
    Ok(ledger.map(|l| l as u32))
}

/// Insert a work item, or update its status/amount if it already exists.
pub async fn upsert_work_item(
    pool: &PgPool,
    item: &WorkItem,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pool_work_items
             (id, attester, amount, status, created_ledger, updated_ledger)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO UPDATE SET
             amount = EXCLUDED.amount,
             status = EXCLUDED.status,
             updated_ledger = EXCLUDED.updated_ledger",
    )
    .bind(&item.id)
    .bind(&item.attester)
    .bind(item.amount)
    .bind(&item.status)
    .bind(item.created_ledger)
    .bind(item.updated_ledger)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert a payout, ignoring duplicates by `id`.
pub async fn insert_payout(pool: &PgPool, payout: &Payout) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pool_payouts
             (id, work_item_id, attester, amount, ledger, tx_hash)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&payout.id)
    .bind(&payout.work_item_id)
    .bind(&payout.attester)
    .bind(payout.amount)
    .bind(payout.ledger)
    .bind(&payout.tx_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch a work item by id.
pub async fn work_item(pool: &PgPool, id: &str) -> Result<Option<WorkItem>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, attester, amount, status, created_ledger, updated_ledger
         FROM pool_work_items WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// List work items, optionally filtered by attester, newest first.
pub async fn work_items(
    pool: &PgPool,
    attester: Option<&str>,
    limit: i64,
) -> Result<Vec<WorkItem>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, attester, amount, status, created_ledger, updated_ledger
         FROM pool_work_items
         WHERE ($1::text IS NULL OR attester = $1)
         ORDER BY created_ledger DESC, id DESC
         LIMIT $2",
    )
    .bind(attester)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// List payouts, optionally filtered by attester, newest first.
pub async fn payouts(
    pool: &PgPool,
    attester: Option<&str>,
    limit: i64,
) -> Result<Vec<Payout>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, work_item_id, attester, amount, ledger, tx_hash
         FROM pool_payouts
         WHERE ($1::text IS NULL OR attester = $1)
         ORDER BY ledger DESC, id DESC
         LIMIT $2",
    )
    .bind(attester)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Checkpoint to store after a page ending at RPC `cursor`.
pub fn checkpoint_after(cursor: &str, events: &[RawEvent], previous_ledger: u32) -> Checkpoint {
    let ledger = cursor_ledger(cursor)
        .or_else(|| events.last().map(|e| e.ledger))
        .unwrap_or(previous_ledger);
    Checkpoint {
        cursor: Some(cursor.to_string()),
        ledger,
        checkpoint_hash: None,
        fencing_token: 0,
    }
}

/// Flags of an attester row first created by a given event.
struct NewAttester {
    allowlisted: bool,
    suspended: bool,
    info_stale: bool,
}

/// Insert an attester row, or apply `set` to the existing one.
async fn upsert_attester(
    tx: &mut Transaction<'_, Postgres>,
    raw: &RawEvent,
    attester: &str,
    new: NewAttester,
    set: &str,
) -> Result<(), sqlx::Error> {
    // `set` is one of the fixed fragments in `apply_event`, never user input.
    let sql = format!(
        "INSERT INTO attesters (address, allowlisted, suspended, info_stale,
                                first_seen_ledger, updated_ledger, updated_event_id)
         VALUES ($1, $2, $3, $4, $5, $5, $6)
         ON CONFLICT (address) DO UPDATE SET {set},
             updated_ledger = EXCLUDED.updated_ledger,
             updated_event_id = EXCLUDED.updated_event_id"
    );
    sqlx::query(&sql)
        .bind(attester)
        .bind(new.allowlisted)
        .bind(new.suspended)
        .bind(new.info_stale)
        .bind(i64::from(raw.ledger))
        .bind(&raw.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn apply_event(
    tx: &mut Transaction<'_, Postgres>,
    raw: &RawEvent,
    event: &LafiyaEvent,
) -> Result<(), sqlx::Error> {
    let ledger = i64::from(raw.ledger);
    match event {
        // `add_attester` resets info but keeps an existing suspension;
        // `remove_attester` clears both.
        LafiyaEvent::AttesterAdded { attester } => {
            let new = NewAttester {
                allowlisted: true,
                suspended: false,
                info_stale: false,
            };
            let set = "allowlisted = true, region = NULL, license_hash = NULL, info_stale = false";
            upsert_attester(tx, raw, attester, new, set).await?;
        }
        LafiyaEvent::AttesterRemoved { attester } => {
            let new = NewAttester {
                allowlisted: false,
                suspended: false,
                info_stale: false,
            };
            let set = "allowlisted = false, suspended = false";
            upsert_attester(tx, raw, attester, new, set).await?;
        }
        LafiyaEvent::AttesterSuspended { attester } => {
            let new = NewAttester {
                allowlisted: true,
                suspended: true,
                info_stale: false,
            };
            upsert_attester(tx, raw, attester, new, "suspended = true").await?;
        }
        LafiyaEvent::AttesterReinstated { attester } => {
            let new = NewAttester {
                allowlisted: true,
                suspended: false,
                info_stale: false,
            };
            upsert_attester(tx, raw, attester, new, "suspended = false").await?;
        }
        LafiyaEvent::AttesterInfoUpdated { attester } => {
            let new = NewAttester {
                allowlisted: true,
                suspended: false,
                info_stale: true,
            };
            upsert_attester(tx, raw, attester, new, "info_stale = true").await?;
        }
        LafiyaEvent::AttestationRecorded {
            record_hash,
            attester,
            timestamp,
        } => {
            sqlx::query(
                "INSERT INTO attestations (event_id, record_hash, attester, attested_at, ledger, tx_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&raw.id)
            .bind(record_hash)
            .bind(attester)
            .bind(*timestamp as i64)
            .bind(ledger)
            .bind(&raw.tx_hash)
            .execute(&mut **tx)
            .await?;
        }
        // `revoke_attestation` deletes the record's whole history on chain.
        LafiyaEvent::AttestationRevoked { record_hash } => {
            sqlx::query(
                "INSERT INTO revocations (event_id, record_hash, ledger, tx_hash)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&raw.id)
            .bind(record_hash)
            .bind(ledger)
            .bind(&raw.tx_hash)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE attestations SET revoked_event_id = $1
                 WHERE record_hash = $2 AND revoked_event_id IS NULL",
            )
            .bind(&raw.id)
            .bind(record_hash)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::PoolWorkItemCreated {
            work_item_id,
            attester,
            amount,
        } => {
            sqlx::query(
                "INSERT INTO pool_work_items
                     (id, attester, amount, status, created_ledger, updated_ledger)
                 VALUES ($1, $2, $3, 'pending', $4, $4)
                 ON CONFLICT (id) DO UPDATE SET
                     amount = EXCLUDED.amount,
                     updated_ledger = EXCLUDED.updated_ledger",
            )
            .bind(work_item_id)
            .bind(attester)
            .bind(*amount as i64)
            .bind(ledger)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::PoolWorkItemApproved { work_item_id } => {
            sqlx::query(
                "UPDATE pool_work_items SET status = 'approved', updated_ledger = $1
                 WHERE id = $2",
            )
            .bind(ledger)
            .bind(work_item_id)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::PoolClaimed {
            work_item_id,
            attester,
            amount,
        } => {
            sqlx::query(
                "UPDATE pool_work_items SET status = 'claimed', updated_ledger = $1
                 WHERE id = $2",
            )
            .bind(ledger)
            .bind(work_item_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO pool_payouts
                     (id, work_item_id, attester, amount, ledger, tx_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&raw.id)
            .bind(work_item_id)
            .bind(attester)
            .bind(*amount as i64)
            .bind(ledger)
            .bind(&raw.tx_hash)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::PoolWithdrawn {
            work_item_id,
            attester,
            amount,
        } => {
            sqlx::query(
                "UPDATE pool_work_items SET status = 'withdrawn', updated_ledger = $1
                 WHERE id = $2",
            )
            .bind(ledger)
            .bind(work_item_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO pool_payouts
                     (id, work_item_id, attester, amount, ledger, tx_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&raw.id)
            .bind(work_item_id)
            .bind(attester)
            .bind(*amount as i64)
            .bind(ledger)
            .bind(&raw.tx_hash)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::PoolPaused { paused } => {
            sqlx::query(
                "INSERT INTO pool_state (contract_id, paused, updated_ledger)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (contract_id) DO UPDATE SET
                     paused = EXCLUDED.paused,
                     updated_ledger = EXCLUDED.updated_ledger",
            )
            .bind(&raw.contract_id)
            .bind(*paused)
            .bind(ledger)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::Admin { name, fields } => {
            sqlx::query(
                "INSERT INTO admin_events (event_id, contract_id, name, fields, ledger, tx_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&raw.id)
            .bind(&raw.contract_id)
            .bind(name)
            .bind(Value::Object(fields.clone()))
            .bind(ledger)
            .bind(&raw.tx_hash)
            .execute(&mut **tx)
            .await?;
        }
        LafiyaEvent::Unknown { .. } => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_secs(1), BACKOFF_BASE_SECS);
        assert_eq!(backoff_secs(2), BACKOFF_BASE_SECS * 2);
        assert_eq!(backoff_secs(3), BACKOFF_BASE_SECS * 4);
        assert_eq!(backoff_secs(100), BACKOFF_MAX_SECS);
    }

    #[test]
    fn retention_defaults_are_bounded() {
        let r = Retention::default();
        assert!(r.batch_size > 0);
        assert!(r.max_batches > 0);
        assert!(r.retention_secs > 0);
    }

    /// Pruning is idempotent and bounded: a second run over an already-pruned
    /// table deletes nothing, and each call issues at most `max_batches`
    /// statements (short locks).
    #[sqlx::test]
    async fn prune_is_idempotent_and_bounded(pool: PgPool) -> sqlx::Result<()> {
        MIGRATOR.run(&pool).await.unwrap();
        let sub = create_subscription(&pool, "https://example.test/hook", "s", None, None, None)
            .await?;
        for i in 0..10 {
            sqlx::query(
                "INSERT INTO deliveries (subscription_id, event_id, created_at)
                 VALUES ($1, $2, now() - interval '30 days')",
            )
            .bind(sub)
            .bind(format!("evt-{i}"))
            .execute(&pool)
            .await?;
        }
        let retention = Retention {
            retention_secs: 60,
            batch_size: 3,
            max_batches: 10,
        };
        let first = prune(&pool, retention).await?;
        assert_eq!(first.deliveries, 10);
        let second = prune(&pool, retention).await?;
        assert_eq!(second.deliveries, 0);
        Ok(())
    }

    /// Enqueueing the same event twice for the same subscription is a no-op.
    #[sqlx::test]
    async fn enqueue_is_idempotent(pool: PgPool) -> sqlx::Result<()> {
        MIGRATOR.run(&pool).await.unwrap();
        let sub = create_subscription(&pool, "https://example.test/hook", "s", None, None, None)
            .await?;
        let mut tx = pool.begin().await?;
        let raw = RawEvent {
            id: "evt-1".into(),
            contract_id: "C".into(),
            ledger: 1,
            ledger_closed_at: "2024-01-01T00:00:00Z".into(),
            tx_hash: "tx".into(),
            topic: vec![],
            value: Value::Null,
            in_successful_contract_call: true,
        };
        let event = LafiyaEvent::AttesterAdded {
            attester: "G".into(),
        };
        assert_eq!(enqueue_deliveries(&mut tx, &raw, &event).await?, 1);
        assert_eq!(enqueue_deliveries(&mut tx, &raw, &event).await?, 0);
        tx.commit().await?;
        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM deliveries WHERE subscription_id = $1")
                .bind(sub)
                .fetch_one(&pool)
                .await?;
        assert_eq!(count, 1);
        Ok(())
    }
}
