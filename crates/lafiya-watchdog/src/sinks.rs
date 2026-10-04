//! Alert sinks: webhooks (Slack, Discord, Matrix, generic), PagerDuty,
//! Opsgenie, and a command sink for email through SMTP (`sendmail`/`msmtp`).

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
            ureq::post("https://events.pagerduty.com/v2/enqueue").send_json(json!({
                "routing_key": routing_key,
                "event_action": "trigger",
                "dedup_key": format!("{}:{}:{}", alert.kind, alert.contract, alert.ledger),
                "payload": {
                    "summary": alert.summary,
                    "source": alert.contract,
                    "severity": pagerduty_severity(alert.severity),
                    "custom_details": alert,
                },
            }))?;
        }
        SinkConfig::Opsgenie { api_key, .. } => {
            ureq::post("https://api.opsgenie.com/v2/alerts")
                .header("Authorization", &format!("GenieKey {api_key}"))
                .send_json(json!({
                    "message": alert.summary,
                    "alias": format!("{}:{}:{}", alert.kind, alert.contract, alert.ledger),
                    "description": alert.text(),
                    "priority": if alert.severity == Severity::Critical { "P1" } else { "P3" },
                }))?;
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
}
