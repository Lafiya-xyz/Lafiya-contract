//! Real-time security watchdog for privileged Lafiya contract events.
//!
//! Mitigations elsewhere (two-step admin transfer, upgrade review, guardian
//! recovery delays) assume someone is watching. This crate is that someone:
//! it ingests contract events from Soroban RPC, evaluates configurable rules
//! ([`rules`]), and fans alerts out to sinks ([`sinks`]). It alerts on its
//! own lag or failure too, so silence always means "nothing happened".

pub mod config;
pub mod rpc;
pub mod rules;
pub mod sinks;

use serde::{Deserialize, Serialize};

/// A decoded contract event, as seen by the rules engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub ledger: u32,
    pub contract: String,
    /// First topic symbol, e.g. `attester_added` (see docs/events.md).
    pub name: String,
    /// Remaining topics rendered as strings (addresses, hex hashes, ...).
    #[serde(default)]
    pub topics: Vec<String>,
    /// Event data rendered as a string.
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub tx_hash: Option<String>,
    /// Ledger close time (RFC 3339), when the source provides it.
    #[serde(default)]
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Severity::Info => "INFO",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        })
    }
}

/// An alert produced by a rule. `kind` is the stable identifier the incident
/// runbooks refer to (e.g. `admin_transferred`, `unknown_wasm_upgrade`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alert {
    pub kind: String,
    pub severity: Severity,
    pub summary: String,
    pub ledger: u32,
    pub contract: String,
    pub details: Vec<String>,
    pub explorer_link: Option<String>,
}

impl Alert {
    /// Plain-text rendering used by every sink.
    pub fn text(&self) -> String {
        let mut s = format!(
            "[{}] {} ({})\ncontract: {}\nledger: {}",
            self.severity, self.summary, self.kind, self.contract, self.ledger
        );
        for d in &self.details {
            s.push('\n');
            s.push_str(d);
        }
        if let Some(link) = &self.explorer_link {
            s.push_str("\nexplorer: ");
            s.push_str(link);
        }
        s
    }
}
