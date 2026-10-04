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
    #[serde(default)]
    pub heartbeat: Heartbeat,
    #[serde(default)]
    pub sinks: Vec<SinkConfig>,
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

fn default_high() -> Severity {
    Severity::High
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
    #[serde(default)]
    pub ping_url: Option<String>,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Heartbeat {
            max_lag_ledgers: default_lag(),
            max_consecutive_failures: default_failures(),
            ping_url: None,
        }
    }
}

fn default_lag() -> u32 {
    60
}

fn default_failures() -> u32 {
    5
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
