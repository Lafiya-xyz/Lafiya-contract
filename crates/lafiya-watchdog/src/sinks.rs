//! Alert sinks: webhooks (Slack, Discord, Matrix, generic), PagerDuty,
//! Opsgenie, and a command sink for email through SMTP (`sendmail`/`msmtp`).
//!
//! Also hosts the dead-man's-switch heartbeat (#532): after each healthy poll
//! the watchdog pings an external monitor (e.g. Healthchecks.io) so that the
//! *absence* of pings pages the operator.

use crate::config::SinkConfig;
use crate::{Alert, Severity};
use serde_json::json;
use std::io::Write;

/// Deliver `alert` to every sink whose `min_severity` it meets. Returns one
/// error string per failed sink; a failing sink never stops the others.
pub fn dispatch(sinks: &[SinkConfig], alert: &Alert) -> Vec<String> {
    sinks
        .iter()
        .filter(|s| alert.severity >= s.min_severity())
        .filter_map(|s| send(s, alert).err().map(|e| e.to_string()))
        .collect()
}

/// Deliver a *group* of related alerts as a single notification per sink.
///
/// Grouping (#532) collapses alerts that share a fingerprint (kind + contract)
/// into one message so a burst does not fan out into N pages. Each sink still
/// honours its `min_severity` against the highest severity in the group.
pub fn dispatch_group(sinks: &[SinkConfig], group: &[Alert]) -> Vec<String> {
    let Some(peak) = group.iter().map(|a| a.severity).max() else {
        return Vec::new();
    };
    let Some(first) = group.first() else {
        return Vec::new();
    };
    let grouped = grouped_alert(first, group);
    sinks
        .iter()
        .filter(|s| peak >= s.min_severity())
        .filter_map(|s| send(s, &grouped).err().map(|e| e.to_string()))
        .collect()
}

/// Collapse a group of alerts into one representative alert whose summary
/// notes the count and whose details list each member's ledger.
pub fn grouped_alert(first: &Alert, group: &[Alert]) -> Alert {
    let mut out = first.clone();
    if group.len() > 1 {
        out.summary = format!("{} ({} related alerts)", first.summary, group.len());
        out.details = group
            .iter()
            .map(|a| format!("ledger {}: {}", a.ledger, a.summary))
            .collect();
    }
    out
}

/// Request body for a webhook `format`.
pub fn webhook_body(format: &str, alert: &Alert) -> serde_json::Value {
    let text = alert.text();
    match format {
        "slack" => json!({ "text": text }),
        "discord" => json!({ "content": text }),
        "matrix" => json!({ "msgtype": "m.text", "body": text }),
        _ => serde_json::to_value(alert).expect("alert serializes"),
    }
}

/// Render the attached pause proposal (#533) as a compact, key-free block.
///
/// The proposal is *unsigned*: it carries the XDR and the auth-entry shape but
/// never any secret key material. We surface only the human-reviewable fields
/// so a webhook/PagerDuty/Opsgenie body cannot leak signer secrets.
pub fn pause_proposal_block(alert: &Alert) -> Option<serde_json::Value> {
    let proposal = alert.pause_proposal.as_ref()?;
    Some(json!({
        "action": "pause",
        "contract": proposal.contract,
        "reason": proposal.reason,
        "unsigned_xdr": proposal.unsigned_xdr,
        "required_signers": proposal.required_signers,
        "expires_at": proposal.expires_at,
        "note": "UNSIGNED — collect guardian signatures via multisig tooling; watchdog holds no keys",
    }))
}

/// Ping an external dead-man's-switch monitor (e.g. Healthchecks.io) with a
/// single GET. Returns an error string on failure so the caller can log it;
/// a failed ping is itself a signal that the watchdog may be degraded.
pub fn heartbeat_ping(url: &str) -> Result<(), String> {
    ureq::get(url)
        .call()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// HMAC-SHA256 signature of `body`, hex-encoded, for the
/// `X-Lafiya-Signature` header (#745).
pub fn sign_payload(key: &[u8], body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Reference verifier for `X-Lafiya-Signature`: recompute the HMAC over the
/// exact received body and compare in constant time.
pub fn verify_signature(key: &[u8], body: &[u8], signature_hex: &str) -> bool {
    let expected = sign_payload(key, body);
    let a = expected.as_bytes();
    let b = signature_hex.trim().to_ascii_lowercase();
    let b = b.as_bytes();
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One queued alert awaiting delivery to a single sink.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutboxEntry {
    pub id: u64,
    /// Index into the configured sink list.
    pub sink: usize,
    pub alert: Alert,
    /// Unix seconds when the entry was first queued.
    pub created_at: u64,
    /// Delivery attempts so far.
    pub attempts: u32,
    /// Unix seconds before which the entry must not be retried.
    pub next_attempt_at: u64,
}

/// Durable alert outbox (#745): alerts survive a sink outage and a watchdog
/// restart. Entries are persisted as JSONL and retried with exponential
/// backoff until `max_age_secs`, after which they are dropped and counted.
#[derive(Debug, Clone)]
pub struct Outbox {
    path: std::path::PathBuf,
    entries: Vec<OutboxEntry>,
    next_id: u64,
    pub base_backoff_secs: u64,
    pub max_backoff_secs: u64,
    pub max_age_secs: u64,
    /// Sink index used for escalation once an entry exceeds `max_age_secs`.
    pub escalation_sink: Option<usize>,
    pub dropped: u64,
    pub escalated: u64,
}

impl Outbox {
    /// Open (or create) the outbox at `path`, reloading any pending entries.
    pub fn open(path: impl Into<std::path::PathBuf>) -> anyhow::Result<Outbox> {
        let path = path.into();
        let mut entries = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                if let Ok(e) = serde_json::from_str::<OutboxEntry>(line) {
                    entries.push(e);
                }
            }
        }
        let next_id = entries.iter().map(|e| e.id + 1).max().unwrap_or(1);
        Ok(Outbox {
            path,
            entries,
            next_id,
            base_backoff_secs: 5,
            max_backoff_secs: 300,
            max_age_secs: 3600,
            escalation_sink: None,
            dropped: 0,
            escalated: 0,
        })
    }

    /// Pending entries, oldest first.
    pub fn entries(&self) -> &[OutboxEntry] {
        &self.entries
    }

    /// Queue `alert` for every sink it meets, then flush to disk.
    pub fn enqueue(&mut self, sinks: &[SinkConfig], alert: &Alert, now: u64) {
        for (idx, sink) in sinks.iter().enumerate() {
            if alert.severity < sink.min_severity() {
                continue;
            }
            self.entries.push(OutboxEntry {
                id: self.next_id,
                sink: idx,
                alert: alert.clone(),
                created_at: now,
                attempts: 0,
                next_attempt_at: now,
            });
            self.next_id += 1;
        }
        self.flush();
    }

    /// Exponential backoff for the given attempt count.
    pub fn backoff_secs(&self, attempts: u32) -> u64 {
        let shift = attempts.saturating_sub(1).min(16);
        self.base_backoff_secs
            .saturating_mul(1u64 << shift)
            .min(self.max_backoff_secs)
    }

    /// Attempt delivery of every due entry. Successful entries are removed;
    /// failures are rescheduled with backoff, or escalated/dropped past
    /// `max_age_secs`. Returns the errors observed this pass.
    pub fn drain(&mut self, sinks: &[SinkConfig], now: u64) -> Vec<String> {
        let mut errors = Vec::new();
        let mut keep = Vec::with_capacity(self.entries.len());
        for mut entry in std::mem::take(&mut self.entries) {
            if entry.next_attempt_at > now {
                keep.push(entry);
                continue;
            }
            if now.saturating_sub(entry.created_at) > self.max_age_secs {
                if let Some(esc) = self.escalation_sink {
                    if entry.sink != esc {
                        if let Some(sink) = sinks.get(esc) {
                            if let Err(e) = send(sink, &entry.alert) {
                                errors.push(format!("escalation sink failed: {e}"));
                            } else {
                                self.escalated += 1;
                                continue;
                            }
                        }
                    }
                }
                self.dropped += 1;
                errors.push(format!(
                    "outbox entry {} for sink {} expired after {}s",
                    entry.id, entry.sink, self.max_age_secs
                ));
                continue;
            }
            let Some(sink) = sinks.get(entry.sink) else {
                self.dropped += 1;
                continue;
            };
            match send(sink, &entry.alert) {
                Ok(()) => {}
                Err(e) => {
                    errors.push(format!("sink {}: {e}", entry.sink));
                    entry.attempts += 1;
                    entry.next_attempt_at = now + self.backoff_secs(entry.attempts);
                    keep.push(entry);
                }
            }
        }
        self.entries = keep;
        self.flush();
        errors
    }

    /// Persist pending entries as JSONL (atomic replace).
    pub fn flush(&self) {
        let tmp = self.path.with_extension("tmp");
        let mut out = String::new();
        for e in &self.entries {
            if let Ok(line) = serde_json::to_string(e) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// Prometheus-style metrics for the outbox (#745).
    pub fn metrics(&self, now: u64) -> String {
        let pending = self.entries.len();
        let oldest = self
            .entries
            .iter()
            .map(|e| now.saturating_sub(e.created_at))
            .max()
            .unwrap_or(0);
        let retries: u32 = self.entries.iter().map(|e| e.attempts).sum();
        format!(
            "# HELP lafiya_watchdog_outbox_pending Alerts awaiting delivery\n\
             # TYPE lafiya_watchdog_outbox_pending gauge\n\
             lafiya_watchdog_outbox_pending {pending}\n\
             # HELP lafiya_watchdog_outbox_oldest_age_seconds Age of the oldest pending alert\n\
             # TYPE lafiya_watchdog_outbox_oldest_age_seconds gauge\n\
             lafiya_watchdog_outbox_oldest_age_seconds {oldest}\n\
             # HELP lafiya_watchdog_outbox_retries_total Delivery attempts that failed\n\
             # TYPE lafiya_watchdog_outbox_retries_total counter\n\
             lafiya_watchdog_outbox_retries_total {retries}\n\
             # HELP lafiya_watchdog_outbox_dropped_total Alerts dropped past max age\n\
             # TYPE lafiya_watchdog_outbox_dropped_total counter\n\
             lafiya_watchdog_outbox_dropped_total {}\n\
             # HELP lafiya_watchdog_outbox_escalated_total Alerts delivered via escalation sink\n\
             # TYPE lafiya_watchdog_outbox_escalated_total counter\n\
             lafiya_watchdog_outbox_escalated_total {}\n",
            self.dropped, self.escalated
        )
    }
}

fn pagerduty_severity(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::High => "error",
        Severity::Info => "info",
    }
}

fn send(sink: &SinkConfig, alert: &Alert) -> anyhow::Result<()> {
    match sink {
        SinkConfig::Webhook {
            url,
            format,
            signing_key,
            ..
        } => {
            let body = webhook_body(format, alert);
            let mut req = ureq::post(url);
            if let Some(key) = signing_key {
                let raw = serde_json::to_vec(&body)?;
                req = req.header("X-Lafiya-Signature", &sign_payload(key.as_bytes(), &raw));
            }
            req.send_json(body)?;
        }
        SinkConfig::Pagerduty { routing_key, .. } => {
            let mut payload = json!({
                "summary": alert.summary,
                "source": alert.contract,
                "severity": pagerduty_severity(alert.severity),
                "custom_details": alert,
            });
            if let Some(proposal) = pause_proposal_block(alert) {
                payload["custom_details"]["pause_proposal"] = proposal;
            }
            ureq::post("https://events.pagerduty.com/v2/enqueue").send_json(json!({
                "routing_key": routing_key,
                "event_action": "trigger",
                "dedup_key": format!("{}:{}:{}", alert.kind, alert.contract, alert.ledger),
                "payload": payload,
            }))?;
        }
        SinkConfig::Opsgenie { api_key, .. } => {
            let mut body = json!({
                "message": alert.summary,
                "alias": format!("{}:{}:{}", alert.kind, alert.contract, alert.ledger),
                "description": alert.text(),
                "priority": if alert.severity == Severity::Critical { "P1" } else { "P3" },
            });
            if let Some(proposal) = pause_proposal_block(alert) {
                body["details"] = json!({ "pause_proposal": proposal });
            }
            ureq::post("https://api.opsgenie.com/v2/alerts")
                .header("Authorization", &format!("GenieKey {api_key}"))
                .send_json(body)?;
        }
        SinkConfig::Command { program, args, .. } => {
            let mut child = std::process::Command::new(program)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                writeln!(
                    stdin,
                    "Subject: [lafiya-watchdog] [{}] {}\n",
                    alert.severity, alert.summary
                )?;
                writeln!(stdin, "{}", alert.text())?;
            }
            let status = child.wait()?;
            if !status.success() {
                anyhow::bail!("{program} exited with {status}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert() -> Alert {
        Alert {
            kind: "admin_transferred".into(),
            severity: Severity::Critical,
            summary: "admin_transferred on CREG".into(),
            ledger: 7,
            contract: "CREG".into(),
            details: vec![],
            explorer_link: None,
        }
    }

    #[test]
    fn webhook_formats() {
        assert!(webhook_body("slack", &alert())["text"]
            .as_str()
            .unwrap()
            .contains("CRITICAL"));
        assert!(webhook_body("discord", &alert())["content"].is_string());
        assert_eq!(webhook_body("matrix", &alert())["msgtype"], "m.text");
        assert_eq!(
            webhook_body("generic", &alert())["kind"],
            "admin_transferred"
        );
    }

    #[test]
    fn severity_filter_skips_low_alerts() {
        let sinks = vec![SinkConfig::Command {
            program: "false".into(),
            args: vec![],
            min_severity: Some(Severity::Critical),
        }];
        let mut low = alert();
        low.severity = Severity::High;
        assert!(
            dispatch(&sinks, &low).is_empty(),
            "filtered sink must not run"
        );
        assert_eq!(
            dispatch(&sinks, &alert()).len(),
            1,
            "failing sink reports an error"
        );
    }

    #[test]
    fn grouped_delivery_collapses_burst() {
        let mut a = alert();
        a.ledger = 7;
        let mut b = alert();
        b.ledger = 8;
        let grouped = grouped_alert(&a, &[a.clone(), b.clone()]);
        assert!(grouped.summary.contains("2 related alerts"));
        assert_eq!(grouped.details.len(), 2);
        assert!(grouped.details[0].contains("ledger 7"));
    }

    #[test]
    fn grouped_dispatch_honours_peak_severity() {
        let sinks = vec![SinkConfig::Command {
            program: "false".into(),
            args: vec![],
            min_severity: Some(Severity::Critical),
        }];
        let mut low = alert();
        low.severity = Severity::High;
        // A group whose peak is only High must be filtered out entirely.
        assert!(dispatch_group(&sinks, &[low.clone(), low]).is_empty());
        // A group containing a Critical must reach the sink (and report its error).
        assert_eq!(dispatch_group(&sinks, &[alert()]).len(), 1);
    }

    #[test]
    fn pause_proposal_rendered_without_keys() {
        let mut a = alert();
        a.pause_proposal = Some(crate::PauseProposal {
            contract: "CREG".into(),
            reason: "admin_transferred".into(),
            unsigned_xdr: "AAAA...".into(),
            required_signers: 2,
            expires_at: 1_700_000_000,
        });
        let block = pause_proposal_block(&a).expect("proposal present");
        assert_eq!(block["action"], "pause");
        assert_eq!(block["required_signers"], 2);
        let rendered = block.to_string();
        assert!(!rendered.contains("secret"), "must not leak key material");
    }

    #[test]
    fn heartbeat_ping_reports_failure() {
        // An unroutable URL must surface an error rather than panic.
        assert!(heartbeat_ping("http://127.0.0.1:1/hb").is_err());
    }
}
