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

fn pagerduty_severity(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::High => "error",
        Severity::Info => "info",
    }
}

fn send(sink: &SinkConfig, alert: &Alert) -> anyhow::Result<()> {
    match sink {
        SinkConfig::Webhook { url, format, .. } => {
            ureq::post(url).send_json(webhook_body(format, alert))?;
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
