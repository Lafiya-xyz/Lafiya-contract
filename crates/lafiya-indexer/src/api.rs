//! Read-only HTTP/JSON API (spec: `openapi.yaml`, served at `/openapi.yaml`).

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::metrics::Metrics;
use crate::store;

pub const OPENAPI: &str = include_str!("../openapi.yaml");

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 500;

/// Header carrying an optional API key.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Abuse-protection configuration. Limits are **on by default**; they can only
/// be turned off by explicitly setting `enabled = false` (e.g. via
/// `LAFIYA_API_LIMITS_DISABLED=1`).
#[derive(Clone, Debug)]
pub struct LimitsConfig {
    /// Master switch. Defaults to `true`.
    pub enabled: bool,
    /// Per-IP bucket: sustained requests per second.
    pub ip_rate_per_sec: f64,
    /// Per-IP bucket: burst capacity.
    pub ip_burst: f64,
    /// Per-key bucket: sustained requests per second.
    pub key_rate_per_sec: f64,
    /// Per-key bucket: burst capacity.
    pub key_burst: f64,
    /// Known API keys mapped to a tier name. Unknown keys are rejected.
    pub keys: HashMap<String, String>,
    /// Per-tier overrides of `(rate_per_sec, burst)`.
    pub tiers: HashMap<String, (f64, f64)>,
    /// Maximum number of tracked buckets before idle ones are evicted.
    pub max_buckets: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ip_rate_per_sec: 10.0,
            ip_burst: 20.0,
            key_rate_per_sec: 50.0,
            key_burst: 100.0,
            keys: HashMap::new(),
            tiers: HashMap::new(),
            max_buckets: 100_000,
        }
    }
}

impl LimitsConfig {
    /// Build from environment variables. Limits stay enabled unless
    /// `LAFIYA_API_LIMITS_DISABLED` is set to a truthy value.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("LAFIYA_API_LIMITS_DISABLED") {
            let v = v.trim().to_ascii_lowercase();
            cfg.enabled = !matches!(v.as_str(), "1" | "true" | "yes" | "on");
        }
        if let Ok(v) = std::env::var("LAFIYA_API_IP_RATE") {
            if let Ok(n) = v.parse() {
                cfg.ip_rate_per_sec = n;
            }
        }
        if let Ok(v) = std::env::var("LAFIYA_API_IP_BURST") {
            if let Ok(n) = v.parse() {
                cfg.ip_burst = n;
            }
        }
        if let Ok(v) = std::env::var("LAFIYA_API_KEY_RATE") {
            if let Ok(n) = v.parse() {
                cfg.key_rate_per_sec = n;
            }
        }
        if let Ok(v) = std::env::var("LAFIYA_API_KEY_BURST") {
            if let Ok(n) = v.parse() {
                cfg.key_burst = n;
            }
        }
        // `LAFIYA_API_KEYS="key1:tierA,key2:tierB"`.
        if let Ok(v) = std::env::var("LAFIYA_API_KEYS") {
            for pair in v.split(',') {
                let pair = pair.trim();
                if pair.is_empty() {
                    continue;
                }
                let (key, tier) = match pair.split_once(':') {
                    Some((k, t)) => (k.trim(), t.trim().to_string()),
                    None => (pair, "default".to_string()),
                };
                if !key.is_empty() {
                    cfg.keys.insert(key.to_string(), tier);
                }
            }
        }
        // `LAFIYA_API_TIERS="tierA:100:200,tierB:10:20"` (rate:burst).
        if let Ok(v) = std::env::var("LAFIYA_API_TIERS") {
            for spec in v.split(',') {
                let parts: Vec<&str> = spec.trim().split(':').collect();
                if parts.len() == 3 {
                    if let (Ok(rate), Ok(burst)) = (parts[1].parse(), parts[2].parse()) {
                        cfg.tiers.insert(parts[0].trim().to_string(), (rate, burst));
                    }
                }
            }
        }
        cfg
    }

    fn tier_for(&self, key: &str) -> (f64, f64) {
        match self.keys.get(key) {
            Some(tier) => self
                .tiers
                .get(tier)
                .copied()
                .unwrap_or((self.key_rate_per_sec, self.key_burst)),
            None => (self.key_rate_per_sec, self.key_burst),
        }
    }
}

/// A single token bucket.
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last: Instant::now(),
        }
    }

    /// Refill by elapsed time and try to take one token.
    fn try_take(&mut self, rate: f64, capacity: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * rate).min(capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Shared rate limiter state.
#[derive(Clone)]
pub struct RateLimiter {
    config: LimitsConfig,
    ip_buckets: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    key_buckets: Arc<Mutex<HashMap<String, Bucket>>>,
}

impl RateLimiter {
    pub fn new(config: LimitsConfig) -> Self {
        Self {
            config,
            ip_buckets: Arc::new(Mutex::new(HashMap::new())),
            key_buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> &LimitsConfig {
        &self.config
    }

    /// Check both the IP and (if present) key bucket. Returns `Ok(())` when the
    /// request may proceed, or `Err(retry_after)` when it must be denied.
    async fn check(&self, ip: IpAddr, key: Option<&str>) -> Result<(), Duration> {
        if !self.config.enabled {
            return Ok(());
        }
        // IP bucket.
        {
            let mut buckets = self.ip_buckets.lock().await;
            if buckets.len() >= self.config.max_buckets {
                buckets.retain(|_, b| b.last.elapsed() < Duration::from_secs(60));
            }
            let bucket = buckets
                .entry(ip)
                .or_insert_with(|| Bucket::new(self.config.ip_burst));
            if !bucket.try_take(self.config.ip_rate_per_sec, self.config.ip_burst) {
                let deficit = 1.0 - bucket.tokens;
                let wait = if self.config.ip_rate_per_sec > 0.0 {
                    deficit / self.config.ip_rate_per_sec
                } else {
                    1.0
                };
                return Err(Duration::from_secs_f64(wait.max(0.001)));
            }
        }
        // Key bucket.
        if let Some(key) = key {
            let (rate, burst) = self.config.tier_for(key);
            let mut buckets = self.key_buckets.lock().await;
            if buckets.len() >= self.config.max_buckets {
                buckets.retain(|_, b| b.last.elapsed() < Duration::from_secs(60));
            }
            let bucket = buckets
                .entry(key.to_string())
                .or_insert_with(|| Bucket::new(burst));
            if !bucket.try_take(rate, burst) {
                let deficit = 1.0 - bucket.tokens;
                let wait = if rate > 0.0 { deficit / rate } else { 1.0 };
                return Err(Duration::from_secs_f64(wait.max(0.001)));
            }
        }
        Ok(())
    }
}

/// Hash a value for structured logging so query hashes are never recorded in
/// cleartext.
fn redact(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{}", hex_prefix(&digest, 8))
}

fn hex_prefix(bytes: &[u8], n: usize) -> String {
    let mut out = String::with_capacity(n * 2);
    for b in bytes.iter().take(n) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Extract the client IP, honouring `X-Forwarded-For` only when the request
/// arrives from a trusted proxy (documented deployment requirement).
fn client_ip(req: &Request, trust_forwarded: bool) -> IpAddr {
    if trust_forwarded {
        if let Some(fwd) = req.headers().get("x-forwarded-for") {
            if let Ok(s) = fwd.to_str() {
                if let Some(first) = s.split(',').next() {
                    if let Ok(ip) = first.trim().parse() {
                        return ip;
                    }
                }
            }
        }
    }
    req.extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

fn api_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Constant-shape denial response: same body/headers regardless of reason so
/// callers cannot distinguish "unknown key" from "rate limited".
fn denial(retry_after: Option<Duration>) -> Response {
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({ "error": "request denied" })),
    )
        .into_response();
    let secs = retry_after
        .map(|d| d.as_secs().max(1).to_string())
        .unwrap_or_else(|| "1".to_string());
    if let Ok(v) = header::HeaderValue::from_str(&secs) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

async fn guard(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let limiter = &s.limiter;
    let ip = client_ip(&req, s.trust_forwarded_for);
    let key = api_key(req.headers()).map(str::to_string);

    // Unknown keys are rejected when a key set is configured.
    if let Some(k) = &key {
        if !limiter.config.keys.is_empty() && !limiter.config.keys.contains_key(k) {
            tracing::warn!(
                event = "api.denied",
                reason = "unknown_key",
                ip = %ip,
                key = %redact(k),
                path = %req.uri().path(),
                "api request denied"
            );
            return denial(None);
        }
    }

    match limiter.check(ip, key.as_deref()).await {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            tracing::warn!(
                event = "api.denied",
                reason = "rate_limited",
                ip = %ip,
                key = key.as_deref().map(redact).unwrap_or_default(),
                path = %req.uri().path(),
                retry_after_ms = retry_after.as_millis() as u64,
                "api request denied"
            );
            denial(Some(retry_after))
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub metrics: Arc<Metrics>,
    pub limiter: RateLimiter,
    /// Whether to trust `X-Forwarded-For` (set when behind a reverse proxy).
    pub trust_forwarded_for: bool,
}

impl AppState {
    pub fn new(pool: PgPool, metrics: Arc<Metrics>, limits: LimitsConfig) -> Self {
        Self {
            pool,
            metrics,
            limiter: RateLimiter::new(limits),
            trust_forwarded_for: std::env::var("LAFIYA_TRUST_PROXY")
                .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let guarded = Router::new()
        .route("/attesters", get(attesters))
        .route("/attestations", get(attestations))
        .route("/records/{hash}/history", get(record_history))
        .route_layer(middleware::from_fn_with_state(state.clone(), guard));
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/openapi.yaml", get(openapi))
        .route("/attesters", get(attesters))
        .route("/attestations", get(attestations))
        .route("/records/{hash}/history", get(record_history))
        .route("/records/{hash}/status", get(record_status))
        .route("/records/{hash}/disputes", get(record_disputes))
        .route("/expiring", get(expiring))
        .route("/status-list", get(status_list_root))
        .route("/status-list/{segment}", get(status_list_segment))
        .with_state(state)
}

pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Db(sqlx::Error),
    Delivery(String),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::Db(e) => {
                tracing::error!(error = %e, "api query failed");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "database unavailable".into(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// A page of results; pass `next_cursor` back as `cursor` for the next one.
///
/// `finalized_ledger` is the highest ledger the indexer considers final; events
/// at or below it will never be rolled back, so consumers can treat them as
/// settled. It is `None` when no finality boundary is known yet.
#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub finalized_ledger: Option<i64>,
}

fn page<T>(mut items: Vec<T>, limit: i64, key: impl Fn(&T) -> String) -> Page<T> {
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(key)
    } else {
        None
    };
    Page {
        items,
        next_cursor,
        finalized_ledger: None,
    }
}

/// Attach the current finality boundary to a page of results.
async fn with_finality<T>(
    pool: &PgPool,
    mut page: Page<T>,
) -> Result<Json<Page<T>>, ApiError> {
    page.finalized_ledger = store::finalized_ledger(pool).await?;
    Ok(Json(page))
}

fn limit(requested: Option<i64>) -> Result<i64, ApiError> {
    match requested.unwrap_or(DEFAULT_LIMIT) {
        l @ 1..=MAX_LIMIT => Ok(l),
        _ => Err(ApiError::BadRequest(format!(
            "limit must be 1..={MAX_LIMIT}"
        ))),
    }
}

fn validate_hash(hash: &str) -> Result<String, ApiError> {
    let hash = hash.to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "hash must be 64 hex characters".into(),
        ));
    }
    Ok(hash)
}

async fn health(State(s): State<AppState>) -> Response {
    match store::checkpoint(&s.pool).await {
        Ok(cp) => Json(json!({
            "status": "ok",
            "checkpoint_ledger": cp.map(|c| c.ledger),
            "lag_ledgers": s.metrics.lag_ledgers.get(),
        }))
        .into_response(),
        Err(e) => ApiError::Db(e).into_response(),
    }
}

async fn metrics(State(s): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render(),
    )
}

async fn openapi() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI)
}

#[derive(Deserialize)]
pub struct AttesterQuery {
    region: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Attester {
    address: String,
    status: String,
    region: Option<String>,
    license_hash: Option<String>,
    first_seen_ledger: i64,
    updated_ledger: i64,
}

async fn attesters(
    State(s): State<AppState>,
    Query(q): Query<AttesterQuery>,
) -> Result<Json<Page<Attester>>, ApiError> {
    let limit = limit(q.limit)?;
    if let Some(status) = &q.status {
        if !["active", "suspended", "removed"].contains(&status.as_str()) {
            return Err(ApiError::BadRequest(
                "status must be active, suspended, or removed".into(),
            ));
        }
    }
    let rows: Vec<Attester> = sqlx::query_as(
        "SELECT address, status, region, license_hash, first_seen_ledger, updated_ledger
         FROM attesters
         WHERE ($1::text IS NULL OR region = $1)
           AND ($2::text IS NULL OR status = $2)
           AND ($3::text IS NULL OR address > $3)
         ORDER BY address LIMIT $4",
    )
    .bind(&q.region)
    .bind(&q.status)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    with_finality(&s.pool, page(rows, limit, |a| a.address.clone())).await
}

#[derive(Deserialize)]
pub struct AttestationQuery {
    attester: Option<String>,
    record_hash: Option<String>,
    /// Only return attestations whose TTL expires at or before this ledger.
    expiring_before: Option<i64>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Attestation {
    event_id: String,
    record_hash: String,
    attester: String,
    timestamp: i64,
    ledger: i64,
    tx_hash: String,
    revoked: bool,
    /// Ledger at which the entry's TTL expires (from the latest TTL bump).
    expires_at_ledger: Option<i64>,
    /// Number of distinct attesters that have co-attested this record.
    quorum_count: Option<i64>,
    /// Whether the record's quorum policy is currently satisfied.
    quorum_satisfied: Option<bool>,
    /// Whether a patient-initiated dispute is currently active.
    disputed: bool,
}

async fn attestations(
    State(s): State<AppState>,
    Query(q): Query<AttestationQuery>,
) -> Result<Json<Page<Attestation>>, ApiError> {
    let limit = limit(q.limit)?;
    let rows: Vec<Attestation> = sqlx::query_as(
        "SELECT a.event_id, a.record_hash, a.attester, a.attested_at AS timestamp, a.ledger,
                a.tx_hash, a.revoked_event_id IS NOT NULL AS revoked,
                t.expires_at_ledger,
                q.quorum_count,
                q.quorum_satisfied,
                COALESCE(d.disputed, FALSE) AS disputed
         FROM attestations a
         LEFT JOIN LATERAL (
             SELECT expires_at_ledger FROM ttl_bumps
             WHERE record_hash = a.record_hash
             ORDER BY ledger DESC LIMIT 1
         ) t ON TRUE
         LEFT JOIN record_quorum q ON q.record_hash = a.record_hash
         LEFT JOIN record_disputes d ON d.record_hash = a.record_hash
         WHERE ($1::text IS NULL OR a.attester = $1)
           AND ($2::text IS NULL OR a.record_hash = $2)
           AND ($3::bigint IS NULL OR t.expires_at_ledger <= $3)
           AND ($4::text IS NULL OR a.event_id > $4)
         ORDER BY a.event_id LIMIT $5",
    )
    .bind(&q.attester)
    .bind(q.record_hash.as_ref().map(|h| h.to_ascii_lowercase()))
    .bind(q.expiring_before)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    with_finality(&s.pool, page(rows, limit, |a| a.event_id.clone())).await
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct HistoryEntry {
    event_id: String,
    /// `attested`, `revoked`, `ttl_extended`, or `disputed`.
    kind: String,
    attester: Option<String>,
    timestamp: Option<i64>,
    ledger: i64,
    tx_hash: String,
}

async fn record_history(
    State(s): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Path(hash): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Page<HistoryEntry>>, ApiError> {
    let limit = limit(q.limit)?;
    let hash = validate_hash(&hash)?;
    let rows: Vec<HistoryEntry> = sqlx::query_as(
        "SELECT * FROM (
             SELECT event_id, 'attested' AS kind, attester, attested_at AS timestamp, ledger, tx_hash
             FROM attestations WHERE record_hash = $1
             UNION ALL
             SELECT event_id, 'revoked', NULL, NULL, ledger, tx_hash
             FROM revocations WHERE record_hash = $1
             UNION ALL
             SELECT event_id, 'ttl_extended', caller, NULL, ledger, tx_hash
             FROM ttl_bumps WHERE record_hash = $1
             UNION ALL
             SELECT event_id, 'disputed', signer, NULL, ledger, tx_hash
             FROM disputes WHERE record_hash = $1
         ) h
         WHERE ($2::text IS NULL OR event_id > $2)
         ORDER BY event_id LIMIT $3",
    )
    .bind(&hash)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    with_finality(&s.pool, page(rows, limit, |h| h.event_id.clone())).await
}

#[derive(Deserialize)]
pub struct WorkItemQuery {
    pool: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct WorkItem {
    work_item_id: String,
    pool: String,
    attester: String,
    /// `pending`, `approved`, `claimed`, or `withdrawn`.
    status: String,
    amount: String,
    created_ledger: i64,
    updated_ledger: i64,
    tx_hash: String,
}

async fn work_items(
    State(s): State<AppState>,
    Query(q): Query<WorkItemQuery>,
) -> Result<Json<Page<WorkItem>>, ApiError> {
    let limit = limit(q.limit)?;
    if let Some(status) = &q.status {
        if !["pending", "approved", "claimed", "withdrawn"].contains(&status.as_str()) {
            return Err(ApiError::BadRequest(
                "status must be pending, approved, claimed, or withdrawn".into(),
            ));
        }
    }
    let rows: Vec<WorkItem> = sqlx::query_as(
        "SELECT work_item_id, pool, attester, status, amount::text AS amount,
                created_ledger, updated_ledger, tx_hash
         FROM work_items
         WHERE ($1::text IS NULL OR pool = $1)
           AND ($2::text IS NULL OR status = $2)
           AND ($3::text IS NULL OR work_item_id > $3)
         ORDER BY work_item_id LIMIT $4",
    )
    .bind(&q.pool)
    .bind(&q.status)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    with_finality(&s.pool, page(rows, limit, |w| w.work_item_id.clone())).await
}

#[derive(Deserialize)]
pub struct PayoutQuery {
    pool: Option<String>,
    attester: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Payout {
    event_id: String,
    pool: String,
    work_item_id: String,
    attester: String,
    amount: String,
    ledger: i64,
    tx_hash: String,
}

async fn payouts(
    State(s): State<AppState>,
    Query(q): Query<PayoutQuery>,
) -> Result<Json<Page<Payout>>, ApiError> {
    let limit = limit(q.limit)?;
    let rows: Vec<Payout> = sqlx::query_as(
        "SELECT event_id, pool, work_item_id, attester, amount::text AS amount, ledger, tx_hash
         FROM payouts
         WHERE ($1::text IS NULL OR pool = $1)
           AND ($2::text IS NULL OR attester = $2)
           AND ($3::text IS NULL OR event_id > $3)
         ORDER BY event_id LIMIT $4",
    )
    .bind(&q.pool)
    .bind(&q.attester)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    with_finality(&s.pool, page(rows, limit, |p| p.event_id.clone())).await
}

/// Aggregated verification view for a single record: quorum, dispute, and TTL state.
#[derive(Serialize, sqlx::FromRow)]
pub struct RecordStatus {
    record_hash: String,
    /// `verified`, `pending`, `revoked`, `disputed`, or `archived`.
    state: String,
    /// Number of distinct attesters currently counted toward quorum.
    quorum_count: i64,
    /// Quorum policy required for this record class (0 if none).
    quorum_required: i64,
    quorum_satisfied: bool,
    disputed: bool,
    revoked: bool,
    /// Ledger at which the entry's TTL expires (from the latest TTL bump).
    expires_at_ledger: Option<i64>,
    /// Whether the entry is past its TTL and may need restoration.
    archived: bool,
}

async fn record_status(
    State(s): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Json<RecordStatus>, ApiError> {
    let hash = validate_hash(&hash)?;
    let row: Option<RecordStatus> = sqlx::query_as(
        "SELECT r.record_hash,
                CASE
                    WHEN r.revoked THEN 'revoked'
                    WHEN COALESCE(d.disputed, FALSE) THEN 'disputed'
                    WHEN COALESCE(q.quorum_satisfied, FALSE) THEN 'verified'
                    WHEN COALESCE(t.expires_at_ledger, 0) > 0
                         AND t.expires_at_ledger <= $2 THEN 'archived'
                    ELSE 'pending'
                END AS state,
                COALESCE(q.quorum_count, 0) AS quorum_count,
                COALESCE(q.quorum_required, 0) AS quorum_required,
                COALESCE(q.quorum_satisfied, FALSE) AS quorum_satisfied,
                COALESCE(d.disputed, FALSE) AS disputed,
                r.revoked,
                t.expires_at_ledger,
                COALESCE(t.expires_at_ledger, 0) > 0
                    AND t.expires_at_ledger <= $2 AS archived
         FROM records r
         LEFT JOIN record_quorum q ON q.record_hash = r.record_hash
         LEFT JOIN record_disputes d ON d.record_hash = r.record_hash
         LEFT JOIN LATERAL (
             SELECT expires_at_ledger FROM ttl_bumps
             WHERE record_hash = r.record_hash
             ORDER BY ledger DESC LIMIT 1
         ) t ON TRUE
         WHERE r.record_hash = $1",
    )
    .bind(&hash)
    .bind(current_ledger(&s.pool).await?)
    .fetch_optional(&s.pool)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound("record not found".into()))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Dispute {
    event_id: String,
    record_hash: String,
    signer: String,
    ledger: i64,
    tx_hash: String,
    /// Whether the dispute has been cleared by an attester or admin.
    cleared: bool,
}

async fn record_disputes(
    State(s): State<AppState>,
    Path(hash): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Page<Dispute>>, ApiError> {
    let limit = limit(q.limit)?;
    let hash = validate_hash(&hash)?;
    let rows: Vec<Dispute> = sqlx::query_as(
        "SELECT event_id, record_hash, signer, ledger, tx_hash,
                cleared_event_id IS NOT NULL AS cleared
         FROM disputes
         WHERE record_hash = $1
           AND ($2::text IS NULL OR event_id > $2)
         ORDER BY event_id LIMIT $3",
    )
    .bind(&hash)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(page(rows, limit, |d| d.event_id.clone())))
}

#[derive(Deserialize)]
pub struct ExpiringQuery {
    /// Upper bound (inclusive) on `expires_at_ledger`; defaults to the current ledger + 1000.
    before_ledger: Option<i64>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ExpiringEntry {
    record_hash: String,
    expires_at_ledger: i64,
    /// Ledgers remaining until expiry (negative if already archived).
    ledgers_remaining: i64,
}

/// Watchdog endpoint: records whose TTL expires at or before `before_ledger`.
async fn expiring(
    State(s): State<AppState>,
    Query(q): Query<ExpiringQuery>,
) -> Result<Json<Page<ExpiringEntry>>, ApiError> {
    let limit = limit(q.limit)?;
    let now = current_ledger(&s.pool).await?;
    let before = q.before_ledger.unwrap_or(now + 1000);
    let rows: Vec<ExpiringEntry> = sqlx::query_as(
        "SELECT t.record_hash, t.expires_at_ledger,
                t.expires_at_ledger - $1 AS ledgers_remaining
         FROM (
             SELECT DISTINCT ON (record_hash) record_hash, expires_at_ledger
             FROM ttl_bumps
             ORDER BY record_hash, ledger DESC
         ) t
         WHERE t.expires_at_ledger <= $2
           AND ($3::text IS NULL OR t.record_hash > $3)
         ORDER BY t.record_hash LIMIT $4",
    )
    .bind(now)
    .bind(before)
    .bind(&q.cursor)
    .bind(limit + 1)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(page(rows, limit, |e| e.record_hash.clone())))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct StatusListRoot {
    /// Monotonic version of the published status list.
    version: i64,
    /// Hex-encoded signed root commitment over all segments.
    root: String,
    /// Number of segments in the list.
    segment_count: i64,
    /// Total number of bits (entries) covered by the list.
    bit_count: i64,
    ledger: i64,
}

async fn status_list_root(
    State(s): State<AppState>,
) -> Result<Json<StatusListRoot>, ApiError> {
    let row: Option<StatusListRoot> = sqlx::query_as(
        "SELECT version, root, segment_count, bit_count, ledger
         FROM status_list_roots
         ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(&s.pool)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound("status list not yet published".into()))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct StatusListSegment {
    segment: i64,
    version: i64,
    /// Base64url-encoded bitstring segment.
    bits: String,
    /// Hex-encoded commitment for this segment.
    commitment: String,
    ledger: i64,
}

async fn status_list_segment(
    State(s): State<AppState>,
    Path(segment): Path<i64>,
) -> Result<Json<StatusListSegment>, ApiError> {
    if segment < 0 {
        return Err(ApiError::BadRequest("segment must be non-negative".into()));
    }
    let row: Option<StatusListSegment> = sqlx::query_as(
        "SELECT segment, version, bits, commitment, ledger
         FROM status_list_segments
         WHERE segment = $1
         ORDER BY version DESC LIMIT 1",
    )
    .bind(segment)
    .fetch_optional(&s.pool)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound("segment not found".into()))
}

async fn current_ledger(pool: &PgPool) -> Result<i64, ApiError> {
    let cp = store::checkpoint(pool).await?;
    Ok(cp.map(|c| c.ledger).unwrap_or(0))
}
