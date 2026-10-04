//! Postgres storage: applying event pages atomically with the checkpoint.

use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::decode::{cursor_ledger, decode, DecodeError, LafiyaEvent, RawEvent};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const LIVE: &str = "live";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Last RPC cursor, or `None` to resume from `ledger` via `startLedger`.
    pub cursor: Option<String>,
    pub ledger: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

pub async fn checkpoint(pool: &PgPool) -> Result<Option<Checkpoint>, sqlx::Error> {
    let row: Option<(Option<String>, i64)> =
        sqlx::query_as("SELECT cursor, ledger FROM checkpoints WHERE name = $1")
            .bind(LIVE)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(cursor, ledger)| Checkpoint {
        cursor,
        ledger: ledger as u32,
    }))
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
            applied += 1;
        }
    }
    sqlx::query(
        "INSERT INTO checkpoints (name, cursor, ledger) VALUES ($1, $2, $3)
         ON CONFLICT (name) DO UPDATE
         SET cursor = EXCLUDED.cursor, ledger = EXCLUDED.ledger, updated_at = now()",
    )
    .bind(LIVE)
    .bind(&next.cursor)
    .bind(i64::from(next.ledger))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(applied)
}

/// Checkpoint to store after a page ending at RPC `cursor`.
pub fn checkpoint_after(cursor: &str, events: &[RawEvent], previous_ledger: u32) -> Checkpoint {
    let ledger = cursor_ledger(cursor)
        .or_else(|| events.last().map(|e| e.ledger))
        .unwrap_or(previous_ledger);
    Checkpoint {
        cursor: Some(cursor.to_string()),
        ledger,
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
