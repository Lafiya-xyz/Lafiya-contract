//! `watchdog.toml`: rules, sinks, and heartbeat settings.

use crate::Severity;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub rpc_url: String,
    /// Contract IDs to watch.
    pub contracts: Vec<String>,
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    /// Transaction explorer prefix; the tx hash is appended.
    #[serde(default)]
    pub explorer_tx_url: Option<String>,
    /// Release manifests whose `contracts[].wasm.sha256` are trusted upgrade targets.
    #[serde(default)]
    pub release_manifests: Vec<String>,
    /// Extra trusted wasm hashes (hex), in addition to the manifests.
    #[serde(default)]
    pub known_wasm_hashes: Vec<String>,
    #[serde(default)]
    pub rules: Vec<EventRule>,
    #[serde(default)]
    pub mass_enrollment: Option<RateRule>,
    /// Alert when attestation entries approach expiry (issue #544).
    #[serde(default)]
    pub expiry_warning: ExpiryWarning,
    /// Alert on patient-initiated dispute flags (issue #546).
    #[serde(default)]
    pub disputes: DisputeMonitoring,
    #[serde(default)]
    pub heartbeat: Heartbeat,
    #[serde(default)]
    pub sinks: Vec<SinkConfig>,
    /// Persistent rule-window / dedup state (#532).
    #[serde(default)]
    pub state: StateConfig,
    /// Escalation policy by severity (#532).
    #[serde(default)]
    pub escalation: Escalation,
    /// Guarded auto-response proposals (#533).
    #[serde(default)]
    pub auto_response: AutoResponse,
}

fn default_poll() -> u64 {
    5
}

/// Alert on every event whose name matches one of `events`. A trailing `*`
/// matches a prefix (e.g. `multisig_*`).
#[derive(Debug, Clone, Deserialize)]
pub struct EventRule {
    pub kind: String,
    pub events: Vec<String>,
    pub severity: Severity,
}

/// Alert when more than `max_count` `event`s land within `window_ledgers`.
#[derive(Debug, Clone, Deserialize)]
pub struct RateRule {
    #[serde(default = "default_rate_event")]
    pub event: String,
    pub max_count: usize,
    pub window_ledgers: u32,
    #[serde(default = "default_high")]
    pub severity: Severity,
}

fn default_rate_event() -> String {
    "attester_added".into()
}

/// Watchdog guidance for attestation TTLs: warn before entries archive so an
/// operator can call `extend_attestation_ttl` (or `restore`) in time.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpiryWarning {
    /// Emit a warning when an entry's remaining TTL falls below this many
    /// ledgers.
    #[serde(default = "default_expiry_warn_ledgers")]
    pub warn_below_ledgers: u32,
    /// Emit a critical alert when remaining TTL falls below this many ledgers.
    #[serde(default = "default_expiry_critical_ledgers")]
    pub critical_below_ledgers: u32,
    /// How often to scan tracked attestation entries, in ledgers.
    #[serde(default = "default_expiry_scan")]
    pub scan_interval_ledgers: u32,
    /// Record hashes (hex) whose TTL should be monitored. Empty means monitor
    /// every entry the indexer reports.
    #[serde(default)]
    pub record_hashes: Vec<String>,
}

impl Default for ExpiryWarning {
    fn default() -> Self {
        ExpiryWarning {
            warn_below_ledgers: default_expiry_warn_ledgers(),
            critical_below_ledgers: default_expiry_critical_ledgers(),
            scan_interval_ledgers: default_expiry_scan(),
            record_hashes: Vec::new(),
        }
    }
}

fn default_expiry_warn_ledgers() -> u32 {
    100_000
}

fn default_expiry_critical_ledgers() -> u32 {
    20_000
}

fn default_expiry_scan() -> u32 {
    1_000
}

/// Watchdog guidance for patient-initiated dispute flags: surface disputes so
/// verifiers and responders act on them.
#[derive(Debug, Clone, Deserialize)]
pub struct DisputeMonitoring {
    /// Alert on every newly observed dispute flag.
    #[serde(default = "default_true")]
    pub alert_on_new: bool,
    /// Alert when a single signer raises more than this many disputes within
    /// `window_ledgers` (anti-abuse / spam signal).
    #[serde(default = "default_dispute_spam")]
    pub max_disputes_per_signer: usize,
    #[serde(default = "default_dispute_window")]
    pub window_ledgers: u32,
    #[serde(default = "default_high")]
    pub severity: Severity,
}

impl Default for DisputeMonitoring {
    fn default() -> Self {
        DisputeMonitoring {
            alert_on_new: true,
            max_disputes_per_signer: default_dispute_spam(),
            window_ledgers: default_dispute_window(),
            severity: Severity::High,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_dispute_spam() -> usize {
    5
}

fn default_dispute_window() -> u32 {
    1_000
}

fn default_high() -> Severity {
    Severity::High
}

/// Thresholds for the incentive-pool rules. Each rule is disabled when its
/// threshold is `None` (or, for the burst rule, when `max_count` is `None`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PoolRules {
    /// Alert when a single withdrawal is at least this many stroops.
    #[serde(default)]
    pub large_withdrawal_stroops: Option<i128>,
    #[serde(default = "default_large_withdrawal_severity")]
    pub large_withdrawal_severity: Severity,
    /// Alert when more than `max_count` approvals land within
    /// `window_ledgers`.
    #[serde(default)]
    pub approval_burst: Option<PoolBurstRule>,
    /// Alert when an attester claims within `window_ledgers` of being
    /// (re-)added.
    #[serde(default)]
    pub claim_after_readd: Option<PoolReaddRule>,
    /// Alert when the pool is paused.
    #[serde(default = "default_pool_paused_severity")]
    pub pool_paused_severity: Severity,
}

fn default_large_withdrawal_severity() -> Severity {
    Severity::High
}

fn default_pool_paused_severity() -> Severity {
    Severity::Critical
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolBurstRule {
    pub max_count: usize,
    #[serde(default = "default_burst_window")]
    pub window_ledgers: u32,
    #[serde(default = "default_high")]
    pub severity: Severity,
}

fn default_burst_window() -> u32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolReaddRule {
    #[serde(default = "default_readd_window")]
    pub window_ledgers: u32,
    #[serde(default = "default_high")]
    pub severity: Severity,
}

fn default_readd_window() -> u32 {
    100
}

#[derive(Debug, Clone, Deserialize)]
pub struct Heartbeat {
    /// Alert when the watchdog falls this many ledgers behind the network.
    #[serde(default = "default_lag")]
    pub max_lag_ledgers: u32,
    /// Alert after this many consecutive failed polls.
    #[serde(default = "default_failures")]
    pub max_consecutive_failures: u32,
    /// Dead-man's switch: pinged (HTTP GET) after every healthy poll, so an
    /// external monitor (healthchecks.io, Cronitor, ...) pages when pings stop.
    /// See `docs/runbooks/watchdog.md` for healthchecks.io / Cronitor setup.
    #[serde(default)]
    pub ping_url: Option<String>,
    /// Minimum seconds between heartbeat pings. Defaults to the poll interval
    /// when unset; a value of `0` pings after every healthy poll.
    #[serde(default)]
    pub ping_interval_secs: Option<u64>,
    /// Optional HTTP method for the ping (`GET` by default). Some monitors
    /// (e.g. Cronitor) accept `POST`.
    #[serde(default)]
    pub ping_method: Option<String>,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Heartbeat {
            max_lag_ledgers: default_lag(),
            max_consecutive_failures: default_failures(),
            ping_url: None,
            ping_interval_secs: None,
            ping_method: None,
        }
    }
}

fn default_lag() -> u32 {
    60
}

fn default_failures() -> u32 {
    5
}

/// Where rule-window and last-alert fingerprints are persisted so a restart
/// neither re-alerts nor loses in-flight windows (#532).
#[derive(Debug, Clone, Deserialize)]
pub struct StateConfig {
    /// Path to the state file. `None` disables persistence (in-memory only).
    #[serde(default)]
    pub path: Option<String>,
    /// Flush the state file after every poll rather than only on shutdown.
    #[serde(default = "default_true")]
    pub flush_every_poll: bool,
}

impl Default for StateConfig {
    fn default() -> Self {
        StateConfig {
            path: None,
            flush_every_poll: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Escalation policy: how long to wait for an ack before re-notifying and
/// which sinks to target per severity (#532).
#[derive(Debug, Clone, Deserialize)]
pub struct Escalation {
    /// Seconds to wait for an acknowledgement before escalating. `0` disables.
    #[serde(default = "default_ack_timeout")]
    pub ack_timeout_secs: u64,
    /// Seconds between re-notifications while unacknowledged. `0` disables.
    #[serde(default = "default_renotify")]
    pub renotify_interval_secs: u64,
    /// Per-severity escalation targets, keyed by `info`/`low`/`medium`/`high`/`critical`.
    #[serde(default)]
    pub targets: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for Escalation {
    fn default() -> Self {
        Escalation {
            ack_timeout_secs: default_ack_timeout(),
            renotify_interval_secs: default_renotify(),
            targets: Default::default(),
        }
    }
}

fn default_ack_timeout() -> u64 {
    900
}

fn default_renotify() -> u64 {
    1800
}

/// Guarded auto-response: build (never execute) pause proposals on critical
/// alerts, gated by rate limiting and alert authenticity checks (#533).
#[derive(Debug, Clone, Deserialize)]
pub struct AutoResponse {
    /// Master switch. Disabled by default; proposals are never executed.
    #[serde(default)]
    pub enabled: bool,
    /// Rule kinds (e.g. `event`, `mass_enrollment`) that may trigger a proposal.
    #[serde(default = "default_critical_kinds")]
    pub critical_kinds: Vec<String>,
    /// Maximum proposals generated per `rate_limit_window_secs`.
    #[serde(default = "default_rate_limit")]
    pub rate_limit: usize,
    /// Window for `rate_limit`, in seconds.
    #[serde(default = "default_rate_window")]
    pub rate_limit_window_secs: u64,
    /// HMAC key (hex) used to verify signed alerts before proposing a pause.
    #[serde(default)]
    pub alert_signing_key: Option<String>,
    /// Public key (hex) for asymmetric alert verification, if used instead of HMAC.
    #[serde(default)]
    pub alert_public_key: Option<String>,
    /// Path to the pause-policy matrix (see `docs/PAUSE-POLICY.md`).
    #[serde(default)]
    pub pause_policy_path: Option<String>,
}

impl Default for AutoResponse {
    fn default() -> Self {
        AutoResponse {
            enabled: false,
            critical_kinds: default_critical_kinds(),
            rate_limit: default_rate_limit(),
            rate_limit_window_secs: default_rate_window(),
            alert_signing_key: None,
            alert_public_key: None,
            pause_policy_path: None,
        }
    }
}

fn default_critical_kinds() -> Vec<String> {
    vec!["event".into(), "mass_enrollment".into()]
}

fn default_rate_limit() -> usize {
    3
}

fn default_rate_window() -> u64 {
    3600
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SinkConfig {
    /// JSON webhook. `format` is `slack`, `discord`, `matrix`, or `generic`.
    Webhook {
        url: String,
        #[serde(default = "default_format")]
        format: String,
        #[serde(default)]
        min_severity: Option<Severity>,
    },
    /// PagerDuty Events API v2.
    Pagerduty {
        routing_key: String,
        #[serde(default)]
        min_severity: Option<Severity>,
    },
    /// Opsgenie Alerts API.
    Opsgenie {
        api_key: String,
        #[serde(default)]
        min_severity: Option<Severity>,
    },
    /// Pipe the alert text to a command's stdin, e.g. `sendmail -t` or
    /// `msmtp oncall@example.org` for email through SMTP.
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        min_severity: Option<Severity>,
    },
}

fn default_format() -> String {
    "generic".into()
}

impl SinkConfig {
    pub fn min_severity(&self) -> Severity {
        match self {
            SinkConfig::Webhook { min_severity, .. }
            | SinkConfig::Pagerduty { min_severity, .. }
            | SinkConfig::Opsgenie { min_severity, .. }
            | SinkConfig::Command { min_severity, .. } => min_severity.unwrap_or(Severity::Info),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text)?;
        let base = path.parent().unwrap_or(Path::new("."));
        for manifest in cfg.release_manifests.clone() {
            cfg.known_wasm_hashes
                .extend(manifest_hashes(&base.join(&manifest))?);
        }
        Ok(cfg)
    }
}

/// Wasm hashes (`contracts[].wasm.sha256`) recorded in a release manifest.
pub fn manifest_hashes(path: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading manifest {}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    Ok(json["contracts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["wasm"]["sha256"].as_str())
        .map(str::to_ascii_lowercase)
        .collect())
}
