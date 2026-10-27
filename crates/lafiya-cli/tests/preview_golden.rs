//! Golden-file tests for the pre-submit state-change preview (issue #536).
//!
//! These tests exercise the preview rendering pipeline for at least five
//! mutating commands. For each command we build a deterministic simulation
//! fixture, render both the human-readable preview and the JSON export, and
//! compare the output against checked-in golden files under
//! `crates/lafiya-cli/tests/golden/preview/`.
//!
//! Golden files can be regenerated with:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p lafiya-cli --test preview_golden
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use lafiya_cli::auth_decode::{AuthNode, AuthTree};
use lafiya_cli::preview::{
    render_preview_json, render_preview_text, BalanceChange, EventChange, LedgerEntryChange,
    Preview, StorageChange,
};
use lafiya_cli::rpc_client::SimulationResult;

/// Directory that holds the checked-in golden files.
fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("preview")
}

/// Compare `actual` against the golden file `name`, or update it when
/// `UPDATE_GOLDEN` is set.
fn assert_golden(name: &str, actual: &str) {
    let path = golden_dir().join(name);

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(golden_dir()).expect("create golden dir");
        fs::write(&path, actual).expect("write golden file");
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read golden file {}: {err}\n\
             run with UPDATE_GOLDEN=1 to generate it",
            path.display()
        )
    });

    assert_eq!(
        expected,
        actual,
        "golden mismatch for {}\n\
         run with UPDATE_GOLDEN=1 to regenerate",
        path.display()
    );
}

/// Build a deterministic simulation result for a given command fixture.
fn fixture(command: &str) -> SimulationResult {
    match command {
        "attester_add" => SimulationResult {
            success: true,
            ledger_entries: vec![
                LedgerEntryChange {
                    key: "attester:GBX7...ATTESTER".to_string(),
                    before: None,
                    after: Some("{\"active\":true,\"added_at\":1700000000}".to_string()),
                },
                LedgerEntryChange {
                    key: "config:attesters".to_string(),
                    before: Some("[\"GAAA...\"]".to_string()),
                    after: Some("[\"GAAA...\",\"GBX7...ATTESTER\"]".to_string()),
                },
            ],
            balance_changes: vec![],
            events: vec![EventChange {
                contract: "attester-registry".to_string(),
                topics: vec!["attester_added".to_string()],
                data: "{\"attester\":\"GBX7...ATTESTER\"}".to_string(),
            }],
            auth: AuthTree::new(vec![AuthNode::leaf(
                "admin",
                "GADMIN...ADMIN",
                vec!["attester_add".to_string()],
            )]),
        },
        "attester_remove" => SimulationResult {
            success: true,
            ledger_entries: vec![
                LedgerEntryChange {
                    key: "attester:GBX7...ATTESTER".to_string(),
                    before: Some("{\"active\":true,\"added_at\":1700000000}".to_string()),
                    after: None,
                },
                LedgerEntryChange {
                    key: "config:attesters".to_string(),
                    before: Some("[\"GAAA...\",\"GBX7...ATTESTER\"]".to_string()),
                    after: Some("[\"GAAA...\"]".to_string()),
                },
            ],
            balance_changes: vec![],
            events: vec![EventChange {
                contract: "attester-registry".to_string(),
                topics: vec!["attester_removed".to_string()],
                data: "{\"attester\":\"GBX7...ATTESTER\"}".to_string(),
            }],
            auth: AuthTree::new(vec![AuthNode::leaf(
                "admin",
                "GADMIN...ADMIN",
                vec!["attester_remove".to_string()],
            )]),
        },
        "admin_propose" => SimulationResult {
            success: true,
            ledger_entries: vec![LedgerEntryChange {
                key: "admin:proposal:7".to_string(),
                before: None,
                after: Some(
                    "{\"action\":\"set_fee\",\"value\":\"100\",\"approvals\":1,\"threshold\":2}"
                        .to_string(),
                ),
            }],
            balance_changes: vec![],
            events: vec![EventChange {
                contract: "admin".to_string(),
                topics: vec!["proposal_created".to_string()],
                data: "{\"id\":7,\"action\":\"set_fee\"}".to_string(),
            }],
            auth: AuthTree::new(vec![AuthNode::leaf(
                "admin",
                "GADMIN...ADMIN",
                vec!["propose".to_string()],
            )]),
        },
        "ops_pause" => SimulationResult {
            success: true,
            ledger_entries: vec![LedgerEntryChange {
                key: "ops:paused".to_string(),
                before: Some("false".to_string()),
                after: Some("true".to_string()),
            }],
            balance_changes: vec![],
            events: vec![EventChange {
                contract: "ops".to_string(),
                topics: vec!["paused".to_string()],
                data: "{\"by\":\"GOPS...OPERATOR\"}".to_string(),
            }],
            auth: AuthTree::new(vec![AuthNode::leaf(
                "operator",
                "GOPS...OPERATOR",
                vec!["pause".to_string()],
            )]),
        },
        "pool_claim" => SimulationResult {
            success: true,
            ledger_entries: vec![
                LedgerEntryChange {
                    key: "pool:claim:GBEN...BENEFICIARY".to_string(),
                    before: Some("{\"claimed\":false}".to_string()),
                    after: Some("{\"claimed\":true}".to_string()),
                },
                LedgerEntryChange {
                    key: "pool:balance".to_string(),
                    before: Some("1000".to_string()),
                    after: Some("750".to_string()),
                },
            ],
            balance_changes: vec![BalanceChange {
                account: "GBEN...BENEFICIARY".to_string(),
                asset: "native".to_string(),
                before: "0".to_string(),
                after: "250".to_string(),
            }],
            events: vec![EventChange {
                contract: "pool".to_string(),
                topics: vec!["claim".to_string()],
                data: "{\"beneficiary\":\"GBEN...BENEFICIARY\",\"amount\":\"250\"}".to_string(),
            }],
            auth: AuthTree::new(vec![AuthNode::leaf(
                "beneficiary",
                "GBEN...BENEFICIARY",
                vec!["claim".to_string()],
            )]),
        },
        other => panic!("unknown fixture: {other}"),
    }
}

/// Render both the text and JSON preview for a fixture and assert against
/// golden files named `<command>.txt` and `<command>.json`.
fn check_command(command: &str) {
    let sim = fixture(command);
    let preview = Preview::from_simulation(&sim);

    let text = render_preview_text(&preview);
    assert_golden(&format!("{command}.txt"), &text);

    let json = render_preview_json(&preview).expect("render json");
    assert_golden(&format!("{command}.json"), &json);
}

#[test]
fn golden_attester_add() {
    check_command("attester_add");
}

#[test]
fn golden_attester_remove() {
    check_command("attester_remove");
}

#[test]
fn golden_admin_propose() {
    check_command("admin_propose");
}

#[test]
fn golden_ops_pause() {
    check_command("ops_pause");
}

#[test]
fn golden_pool_claim() {
    check_command("pool_claim");
}

/// Sanity check: the JSON export must be valid JSON and round-trip the
/// storage changes we fed in. This guards against the golden files silently
/// drifting into an unparseable shape.
#[test]
fn json_export_is_valid_and_round_trips() {
    let sim = fixture("pool_claim");
    let preview = Preview::from_simulation(&sim);
    let json = render_preview_json(&preview).expect("render json");

    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");

    let storage = value
        .get("storage")
        .and_then(|v| v.as_array())
        .expect("storage array present");
    assert_eq!(storage.len(), sim.ledger_entries.len());

    let balances = value
        .get("balances")
        .and_then(|v| v.as_array())
        .expect("balances array present");
    assert_eq!(balances.len(), sim.balance_changes.len());

    let events = value
        .get("events")
        .and_then(|v| v.as_array())
        .expect("events array present");
    assert_eq!(events.len(), sim.events.len());
}

/// The text preview must mention every storage key and event topic so that
/// operators can spot wrong-address / wrong-network mistakes at a glance.
#[test]
fn text_preview_mentions_all_changes() {
    let sim = fixture("pool_claim");
    let preview = Preview::from_simulation(&sim);
    let text = render_preview_text(&preview);

    for entry in &sim.ledger_entries {
        assert!(
            text.contains(&entry.key),
            "text preview missing storage key {}",
            entry.key
        );
    }
    for event in &sim.events {
        for topic in &event.topics {
            assert!(
                text.contains(topic),
                "text preview missing event topic {topic}"
            );
        }
    }
    for balance in &sim.balance_changes {
        assert!(
            text.contains(&balance.account),
            "text preview missing balance account {}",
            balance.account
        );
    }
}

/// A failed simulation must not produce a preview that looks submittable.
#[test]
fn failed_simulation_is_flagged() {
    let mut sim = fixture("ops_pause");
    sim.success = false;
    let preview = Preview::from_simulation(&sim);
    let text = render_preview_text(&preview);
    assert!(
        text.to_lowercase().contains("fail") || text.to_lowercase().contains("error"),
        "failed simulation preview should be flagged: {text}"
    );
}

// Keep the unused import warning away when StorageChange is only used in
// fixtures above via the re-exported type.
#[allow(dead_code)]
fn _assert_storage_change_type(_: StorageChange) {}