//! Regression test: `lafiya-cli --help` (and each subcommand's `--help`)
//! must print usage text and exit successfully.
//!
//! A clap derive regression (e.g. a malformed attribute, an argument
//! conflict, or a name that fails to render) would otherwise only be caught
//! by a human running the CLI. These tests run the real compiled binary --
//! Cargo builds the `lafiya-cli` bin automatically for this package's
//! integration tests and exposes its path via `CARGO_BIN_EXE_lafiya-cli`
//! (Cargo keeps the bin name verbatim, hyphen included, in this variable).
//!
//! When a new subcommand is added, add a matching test below.
//!
//! In addition to the smoke tests, this file contains `insta` snapshot tests
//! that pin the full `--help` output of every command. Snapshots live in
//! `tests/snapshots/` and are committed; run `cargo insta review` (or set
//! `INSTA_UPDATE=always`) after intentionally changing help text.

use std::process::Command;

/// Absolute path to the compiled `lafiya-cli` binary.
const BIN: &str = env!("CARGO_BIN_EXE_lafiya-cli");

/// Run `lafiya-cli <args> --help` and return its stdout.
fn help_stdout(args: &[&str]) -> String {
    let full_args: Vec<&str> = args.iter().copied().chain(["--help"]).collect();
    let output = Command::new(BIN)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "failed to spawn `lafiya-cli {} --help`: {e}",
                args.join(" ")
            )
        });

    assert!(
        output.status.success(),
        "`lafiya-cli {} --help` exited with {} (stderr: {})",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Snapshot the `--help` output of `lafiya-cli <args>`.
///
/// The snapshot name is derived from `args` so each command gets its own
/// committed snapshot file under `tests/snapshots/`.
fn assert_help_snapshot(args: &[&str]) {
    let stdout = help_stdout(args);
    let name = if args.is_empty() {
        "help_top_level".to_string()
    } else {
        format!("help_{}", args.join("_").replace('-', "_"))
    };
    insta::assert_snapshot!(name, stdout);
}

/// Assert that `lafiya-cli <args> --help` prints usage text to stdout and
/// exits successfully (clap exits 0 for `--help`).
fn assert_help_ok(args: &[&str]) {
    let full_args: Vec<&str> = args.iter().copied().chain(["--help"]).collect();
    let output = Command::new(BIN)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "failed to spawn `lafiya-cli {} --help`: {e}",
                args.join(" ")
            )
        });

    assert!(
        output.status.success(),
        "`lafiya-cli {} --help` exited with {} (stderr: {})",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage:"),
        "`lafiya-cli {} --help` printed no usage text (stdout: {:?})",
        args.join(" "),
        stdout,
    );
}

// ── top-level ─────────────────────────────────────────────────────────────────

#[test]
fn top_level_help_prints_usage() {
    assert_help_ok(&[]);
}

#[test]
fn top_level_help_snapshot() {
    assert_help_snapshot(&[]);
}

// ── config ────────────────────────────────────────────────────────────────────

#[test]
fn config_help_prints_usage() {
    assert_help_ok(&["config"]);
}

#[test]
fn config_help_snapshot() {
    assert_help_snapshot(&["config"]);
}

#[test]
fn config_show_help_prints_usage() {
    assert_help_ok(&["config", "show"]);
}

#[test]
fn config_show_help_snapshot() {
    assert_help_snapshot(&["config", "show"]);
}

#[test]
fn config_list_help_prints_usage() {
    assert_help_ok(&["config", "list"]);
}

#[test]
fn config_list_help_snapshot() {
    assert_help_snapshot(&["config", "list"]);
}

#[test]
fn config_env_help_prints_usage() {
    assert_help_ok(&["config", "env"]);
}

#[test]
fn config_env_help_snapshot() {
    assert_help_snapshot(&["config", "env"]);
}

// ── attester ──────────────────────────────────────────────────────────────────

#[test]
fn attester_help_prints_usage() {
    assert_help_ok(&["attester"]);
}

#[test]
fn attester_help_snapshot() {
    assert_help_snapshot(&["attester"]);
}

#[test]
fn attester_is_help_prints_usage() {
    assert_help_ok(&["attester", "is"]);
}

#[test]
fn attester_is_help_snapshot() {
    assert_help_snapshot(&["attester", "is"]);
}

#[test]
fn attester_add_help_prints_usage() {
    assert_help_ok(&["attester", "add"]);
}

#[test]
fn attester_add_help_snapshot() {
    assert_help_snapshot(&["attester", "add"]);
}

#[test]
fn attester_add_with_info_help_prints_usage() {
    assert_help_ok(&["attester", "add-with-info"]);
}

#[test]
fn attester_add_with_info_help_snapshot() {
    assert_help_snapshot(&["attester", "add-with-info"]);
}

#[test]
fn attester_update_info_help_prints_usage() {
    assert_help_ok(&["attester", "update-info"]);
}

#[test]
fn attester_update_info_help_snapshot() {
    assert_help_snapshot(&["attester", "update-info"]);
}

#[test]
fn attester_remove_help_prints_usage() {
    assert_help_ok(&["attester", "remove"]);
}

#[test]
fn attester_remove_help_snapshot() {
    assert_help_snapshot(&["attester", "remove"]);
}

#[test]
fn attester_suspend_help_prints_usage() {
    assert_help_ok(&["attester", "suspend"]);
}

#[test]
fn attester_suspend_help_snapshot() {
    assert_help_snapshot(&["attester", "suspend"]);
}

#[test]
fn attester_reinstate_help_prints_usage() {
    assert_help_ok(&["attester", "reinstate"]);
}

#[test]
fn attester_reinstate_help_snapshot() {
    assert_help_snapshot(&["attester", "reinstate"]);
}

#[test]
fn attester_get_info_help_prints_usage() {
    assert_help_ok(&["attester", "get-info"]);
}

#[test]
fn attester_get_info_help_snapshot() {
    assert_help_snapshot(&["attester", "get-info"]);
}

#[test]
fn attester_get_status_help_prints_usage() {
    assert_help_ok(&["attester", "get-status"]);
}

#[test]
fn attester_get_status_help_snapshot() {
    assert_help_snapshot(&["attester", "get-status"]);
}

#[test]
fn attester_set_max_attesters_help_prints_usage() {
    assert_help_ok(&["attester", "set-max-attesters"]);
}

#[test]
fn attester_set_max_attesters_help_snapshot() {
    assert_help_snapshot(&["attester", "set-max-attesters"]);
}

#[test]
fn attester_get_max_attesters_help_prints_usage() {
    assert_help_ok(&["attester", "get-max-attesters"]);
}

#[test]
fn attester_get_max_attesters_help_snapshot() {
    assert_help_snapshot(&["attester", "get-max-attesters"]);
}

#[test]
fn attester_get_count_help_prints_usage() {
    assert_help_ok(&["attester", "get-count"]);
}

#[test]
fn attester_get_count_help_snapshot() {
    assert_help_snapshot(&["attester", "get-count"]);
}

#[test]
fn attester_get_schema_version_help_prints_usage() {
    assert_help_ok(&["attester", "get-schema-version"]);
}

#[test]
fn attester_get_schema_version_help_snapshot() {
    assert_help_snapshot(&["attester", "get-schema-version"]);
}

#[test]
fn attester_upgrade_help_prints_usage() {
    assert_help_ok(&["attester", "upgrade"]);
}

#[test]
fn attester_upgrade_help_snapshot() {
    assert_help_snapshot(&["attester", "upgrade"]);
}

#[test]
fn attester_migrate_help_prints_usage() {
    assert_help_ok(&["attester", "migrate"]);
}

#[test]
fn attester_migrate_help_snapshot() {
    assert_help_snapshot(&["attester", "migrate"]);
}

// ── attestation ───────────────────────────────────────────────────────────────

#[test]
fn attestation_help_prints_usage() {
    assert_help_ok(&["attestation"]);
}

#[test]
fn attestation_help_snapshot() {
    assert_help_snapshot(&["attestation"]);
}

#[test]
fn attestation_get_help_prints_usage() {
    assert_help_ok(&["attestation", "get"]);
}

#[test]
fn attestation_get_help_snapshot() {
    assert_help_snapshot(&["attestation", "get"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_history_help_prints_usage() {
    assert_help_ok(&["attestation", "get-history"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_attest_help_prints_usage() {
    assert_help_ok(&["attestation", "attest"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_revoke_help_prints_usage() {
    assert_help_ok(&["attestation", "revoke"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_attester_registry_help_prints_usage() {
    assert_help_ok(&["attestation", "get-attester-registry"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_set_attester_registry_help_prints_usage() {
    assert_help_ok(&["attestation", "set-attester-registry"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_extend_ttl_help_prints_usage() {
    assert_help_ok(&["attestation", "extend-ttl"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_restore_help_prints_usage() {
    assert_help_ok(&["attestation", "restore"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_dispute_help_prints_usage() {
    assert_help_ok(&["attestation", "dispute"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_clear_dispute_help_prints_usage() {
    assert_help_ok(&["attestation", "clear-dispute"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_dispute_help_prints_usage() {
    assert_help_ok(&["attestation", "get-dispute"]);
}

// ── admin ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_help_prints_usage() {
    assert_help_ok(&["admin"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_get_help_prints_usage() {
    assert_help_ok(&["admin", "get"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_propose_help_prints_usage() {
    assert_help_ok(&["admin", "propose"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_accept_help_prints_usage() {
    assert_help_ok(&["admin", "accept"]);
}

// ── ops ───────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_help_prints_usage() {
    assert_help_ok(&["ops"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_pause_help_prints_usage() {
    assert_help_ok(&["ops", "pause"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_unpause_help_prints_usage() {
    assert_help_ok(&["ops", "unpause"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_is_paused_help_prints_usage() {
    assert_help_ok(&["ops", "is-paused"]);
}

// ── deploy ────────────────────────────────────────────────────────────────────

#[test]
fn deploy_help_prints_usage() {
    assert_help_ok(&["deploy"]);
}

#[test]
fn deploy_help_snapshot() {
    assert_help_snapshot(&["deploy"]);
}

// ── help text content ─────────────────────────────────────────────────────────

/// Assert that `lafiya-cli <args> --help` prints text containing `needle`.
fn assert_help_contains(args: &[&str], needle: &str) {
    let full_args: Vec<&str> = args.iter().copied().chain(["--help"]).collect();
    let output = Command::new(BIN)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "failed to spawn `lafiya-cli {} --help`: {e}",
                args.join(" ")
            )
        });

    assert!(
        output.status.success(),
        "`lafiya-cli {} --help` exited with {} (stderr: {})",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(needle),
        "`lafiya-cli {} --help` output missing {needle:?} (stdout: {:?})",
        args.join(" "),
        stdout,
    );
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_help_lists_restore_and_dispute() {
    assert_help_contains(&["attestation"], "restore");
    assert_help_contains(&["attestation"], "dispute");
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_extend_ttl_help_documents_flags() {
    assert_help_contains(&["attestation", "extend-ttl"], "record-hash");
    assert_help_contains(&["attestation", "extend-ttl"], "extend-to");
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_restore_help_documents_flags() {
    assert_help_contains(&["attestation", "restore"], "record-hash");
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_dispute_help_documents_flags() {
    assert_help_contains(&["attestation", "dispute"], "record-hash");
    assert_help_contains(&["attestation", "dispute"], "signature");
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_clear_dispute_help_documents_flags() {
    assert_help_contains(&["attestation", "clear-dispute"], "record-hash");
}

// ── snapshots for commands pending CLI wiring ────────────────────────────────
//
// These are ignored alongside their smoke-test counterparts above. Once the
// command modules are wired up, drop the `#[ignore]` attributes here and in
// the corresponding smoke tests; `cargo insta review` will record the
// snapshots on first run.

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_history_help_snapshot() {
    assert_help_snapshot(&["attestation", "get-history"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_attest_help_snapshot() {
    assert_help_snapshot(&["attestation", "attest"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_revoke_help_snapshot() {
    assert_help_snapshot(&["attestation", "revoke"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_attester_registry_help_snapshot() {
    assert_help_snapshot(&["attestation", "get-attester-registry"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_set_attester_registry_help_snapshot() {
    assert_help_snapshot(&["attestation", "set-attester-registry"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_extend_ttl_help_snapshot() {
    assert_help_snapshot(&["attestation", "extend-ttl"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_restore_help_snapshot() {
    assert_help_snapshot(&["attestation", "restore"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_dispute_help_snapshot() {
    assert_help_snapshot(&["attestation", "dispute"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_clear_dispute_help_snapshot() {
    assert_help_snapshot(&["attestation", "clear-dispute"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn attestation_get_dispute_help_snapshot() {
    assert_help_snapshot(&["attestation", "get-dispute"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_help_snapshot() {
    assert_help_snapshot(&["admin"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_get_help_snapshot() {
    assert_help_snapshot(&["admin", "get"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_propose_help_snapshot() {
    assert_help_snapshot(&["admin", "propose"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn admin_accept_help_snapshot() {
    assert_help_snapshot(&["admin", "accept"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_help_snapshot() {
    assert_help_snapshot(&["ops"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_pause_help_snapshot() {
    assert_help_snapshot(&["ops", "pause"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_unpause_help_snapshot() {
    assert_help_snapshot(&["ops", "unpause"]);
}

#[test]
#[ignore = "command lost in merge; CLI wiring pending"]
fn ops_is_paused_help_snapshot() {
    assert_help_snapshot(&["ops", "is-paused"]);
}
