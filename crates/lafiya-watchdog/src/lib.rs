//! Real-time security watchdog for privileged Lafiya contract events.
//!
//! Mitigations elsewhere (two-step admin transfer, upgrade review, guardian
//! recovery delays) assume someone is watching. This crate is that someone:
//! it ingests contract events from Soroban RPC, evaluates configurable rules
//! ([`rules`]), and fans alerts out to sinks ([`sinks`]). It alerts on its
//! own lag or failure too, so silence always means "nothing happened".
//!
//! Reliability features for an on-call rotation:
//! - [`state`] persists rule window state and last-alert fingerprints across
//!   restarts, so a crash never re-alerts or loses an in-flight window.
//! - [`heartbeat`] emits a periodic dead-man's-switch ping to an external
//!   monitor; absence of the ping pages the operator.
//! - [`proposal`] builds unsigned pause proposals on critical alerts for
//!   guardian multisig signing. The watchdog never holds keys and never
//!   executes a pause itself.

pub mod config;
pub mod heartbeat;
pub mod proposal;
pub mod rpc;
pub mod rules;
pub mod sinks;
pub mod state;

pub use heartbeat::{Heartbeat, HeartbeatConfig};
pub use proposal::{PauseProposal, ProposalPolicy};
pub use state::{AlertFingerprint, PersistentState, RuleWindowState, StateStore};

pub use rules::{
    ApprovalBurstRule, ClaimByReaddedAttesterRule, LargeWithdrawalRule, PoolPausedRule,
};

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
