//! Privilege-graph monitoring (#744).
//!
//! Models principals-to-capabilities across every role-bearing contract,
//! updates the graph from events, seeds it from contract reads at startup,
//! and alerts whenever an event creates a *new effective admin path* — i.e. a
//! principal that could not reach an admin capability before the event can
//! reach it after.
//!
//! The graph is a set of edges `(principal, capability, contract)`. Edges are
//! derived from the same event names the rules engine already understands
//! (`admin_transferred`, `signers_*`, `approver_set_changed`, ...), so the
//! monitor needs no new RPC surface beyond the existing event stream.

use crate::{Alert, Event, Severity};
use std::collections::{BTreeMap, BTreeSet};

/// Alert kind emitted when an event grants a principal a new admin path.
pub const NEW_ADMIN_PATH: &str = "new_admin_path";

/// Capabilities that amount to administrative control of a contract.
pub const ADMIN_CAPABILITIES: &[&str] = &[
    "admin",
    "upgrade",
    "pause",
    "unpause",
    "set_approver",
    "set_rate_limit",
    "set_threshold",
    "add_signer",
    "remove_signer",
    "set_recovery",
    "repoint_registry",
    "set_tier",
    "set_bond",
];

/// A single grant: `principal` may exercise `capability` on `contract`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub principal: String,
    pub capability: String,
    pub contract: String,
}

/// The privilege graph plus the set of admin paths already known, so only
/// genuinely new paths alert.
#[derive(Debug, Default, Clone)]
pub struct PrivilegeGraph {
    edges: BTreeSet<Edge>,
    /// `(principal, capability)` pairs already reported, to avoid re-alerting
    /// on every subsequent event that re-grants the same path.
    known_paths: BTreeSet<(String, String)>,
}

impl PrivilegeGraph {
    pub fn new() -> PrivilegeGraph {
        PrivilegeGraph::default()
    }

    /// Seed the graph from a contract read at startup (e.g. `get_admin`,
    /// `get_signers`). Seeded edges never alert: they are the baseline.
    pub fn seed(&mut self, contract: &str, principal: &str, capability: &str) {
        self.insert(contract, principal, capability);
        self.known_paths
            .insert((principal.to_string(), capability.to_string()));
    }

    fn insert(&mut self, contract: &str, principal: &str, capability: &str) {
        if principal.is_empty() {
            return;
        }
        self.edges.insert(Edge {
            principal: principal.to_string(),
            capability: capability.to_string(),
            contract: contract.to_string(),
        });
    }

    /// Every principal that can reach `capability` on `contract`.
    pub fn principals_with(&self, contract: &str, capability: &str) -> Vec<String> {
        self.edges
            .iter()
            .filter(|e| e.contract == contract && e.capability == capability)
            .map(|e| e.principal.clone())
            .collect()
    }

    /// All admin-capability edges, sorted, for the daily snapshot.
    pub fn admin_edges(&self) -> Vec<Edge> {
        self.edges
            .iter()
            .filter(|e| ADMIN_CAPABILITIES.contains(&e.capability.as_str()))
            .cloned()
            .collect()
    }

    /// Apply one event. Returns an alert when the event creates a new
    /// effective admin path, with an explanation of the path.
    pub fn apply(&mut self, e: &Event) -> Option<Alert> {
        let grants = grants_for(e);
        if grants.is_empty() {
            return None;
        }
        let mut new_paths = Vec::new();
        for (principal, capability) in grants {
            self.insert(&e.contract, &principal, &capability);
            if ADMIN_CAPABILITIES.contains(&capability.as_str())
                && !self
                    .known_paths
                    .contains(&(principal.clone(), capability.clone()))
            {
                self.known_paths
                    .insert((principal.clone(), capability.clone()));
                new_paths.push((principal, capability));
            }
        }
        if new_paths.is_empty() {
            return None;
        }
        let mut details: Vec<String> = new_paths
            .iter()
            .map(|(p, c)| format!("new path: {p} -> {c} on {}", e.contract))
            .collect();
        if !e.topics.is_empty() {
            details.push(format!("topics: {}", e.topics.join(", ")));
        }
        if let Some(tx) = &e.tx_hash {
            details.push(format!("tx: {tx}"));
        }
        let summary = new_paths
            .iter()
            .map(|(p, c)| format!("{p} can now {c}"))
            .collect::<Vec<_>>()
            .join("; ");
        Some(Alert {
            kind: NEW_ADMIN_PATH.into(),
            severity: Severity::Critical,
            summary: format!("{} on {}: {summary}", e.name, e.contract),
            ledger: e.ledger,
            contract: e.contract.clone(),
            details,
            explorer_link: None,
        })
    }

    /// Daily snapshot for the governance feed: every admin edge, grouped by
    /// contract, as a stable JSON document.
    pub fn snapshot(&self, ledger: u32) -> serde_json::Value {
        let mut by_contract: BTreeMap<&str, Vec<serde_json::Value>> = BTreeMap::new();
        for edge in self.admin_edges() {
            by_contract
                .entry(edge.contract.as_str())
                .or_default()
                .push(serde_json::json!({
                    "principal": edge.principal,
                    "capability": edge.capability,
                }));
        }
        serde_json::json!({
            "kind": "privilege_graph_snapshot",
            "ledger": ledger,
            "contracts": by_contract,
        })
    }
}

/// Map an event to the `(principal, capability)` grants it implies. The
/// principal is the first topic (the address the event names); events that
/// carry no address grant nothing.
fn grants_for(e: &Event) -> Vec<(String, String)> {
    let principal = match e.topics.first() {
        Some(p) if !p.is_empty() => p.clone(),
        _ => return Vec::new(),
    };
    let capability = match e.name.as_str() {
        "admin_transferred" | "admin_set" => "admin",
        "upgraded" => "upgrade",
        "paused" | "unpaused" => "pause",
        "attester_registry_repointed" => "repoint_registry",
        "approver_set_changed" => "set_approver",
        "rate_limit_set" => "set_rate_limit",
        "signers_added" | "signer_added" => "add_signer",
        "signers_removed" | "signer_removed" => "remove_signer",
        "threshold_set" | "threshold_changed" => "set_threshold",
        "recovery_set" | "recovery_changed" => "set_recovery",
        "tier_set" | "tier_changed" => "set_tier",
        "bond_set" | "bond_changed" => "set_bond",
        _ => return Vec::new(),
    };
    vec![(principal, capability.to_string())]
}
