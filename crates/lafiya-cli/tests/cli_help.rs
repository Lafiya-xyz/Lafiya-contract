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

use std::process::Command;

/// Absolute path to the compiled `lafiya-cli` binary.
const BIN: &str = env!("CARGO_BIN_EXE_lafiya-cli");

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

// ── config ────────────────────────────────────────────────────────────────────

#[test]
fn config_help_prints_usage() {
    assert_help_ok(&["config"]);
}

#[test]
fn config_show_help_prints_usage() {
    assert_help_ok(&["config", "show"]);
}

#[test]
fn config_list_help_prints_usage() {
    assert_help_ok(&["config", "list"]);
}

#[test]
fn config_env_help_prints_usage() {
    assert_help_ok(&["config", "env"]);
}

// ── attester ──────────────────────────────────────────────────────────────────

#[test]
fn attester_help_prints_usage() {
    assert_help_ok(&["attester"]);
}

#[test]
fn attester_is_help_prints_usage() {
    assert_help_ok(&["attester", "is"]);
}

#[test]
fn attester_add_help_prints_usage() {
    assert_help_ok(&["attester", "add"]);
}

#[test]
fn attester_add_with_info_help_prints_usage() {
    assert_help_ok(&["attester", "add-with-info"]);
}

#[test]
fn attester_update_info_help_prints_usage() {
    assert_help_ok(&["attester", "update-info"]);
}

#[test]
fn attester_remove_help_prints_usage() {
    assert_help_ok(&["attester", "remove"]);
}

#[test]
fn attester_suspend_help_prints_usage() {
    assert_help_ok(&["attester", "suspend"]);
}

#[test]
fn attester_reinstate_help_prints_usage() {
    assert_help_ok(&["attester", "reinstate"]);
}

#[test]
fn attester_get_info_help_prints_usage() {
    assert_help_ok(&["attester", "get-info"]);
}

#[test]
fn attester_get_status_help_prints_usage() {
    assert_help_ok(&["attester", "get-status"]);
}

#[test]
fn attester_set_max_attesters_help_prints_usage() {
    assert_help_ok(&["attester", "set-max-attesters"]);
}

#[test]
fn attester_get_max_attesters_help_prints_usage() {
    assert_help_ok(&["attester", "get-max-attesters"]);
}

#[test]
fn attester_get_count_help_prints_usage() {
    assert_help_ok(&["attester", "get-count"]);
}

#[test]
fn attester_get_schema_version_help_prints_usage() {
    assert_help_ok(&["attester", "get-schema-version"]);
}

#[test]
fn attester_upgrade_help_prints_usage() {
    assert_help_ok(&["attester", "upgrade"]);
}

#[test]
fn attester_migrate_help_prints_usage() {
    assert_help_ok(&["attester", "migrate"]);
}

// ── attestation ───────────────────────────────────────────────────────────────

#[test]
fn attestation_help_prints_usage() {
    assert_help_ok(&["attestation"]);
}

#[test]
fn attestation_get_help_prints_usage() {
    assert_help_ok(&["attestation", "get"]);
}

#[test]
fn attestation_get_history_help_prints_usage() {
    assert_help_ok(&["attestation", "get-history"]);
}

#[test]
fn attestation_attest_help_prints_usage() {
    assert_help_ok(&["attestation", "attest"]);
}

#[test]
fn attestation_revoke_help_prints_usage() {
    assert_help_ok(&["attestation", "revoke"]);
}

#[test]
fn attestation_get_attester_registry_help_prints_usage() {
    assert_help_ok(&["attestation", "get-attester-registry"]);
}

#[test]
fn attestation_set_attester_registry_help_prints_usage() {
    assert_help_ok(&["attestation", "set-attester-registry"]);
}

// ── admin ─────────────────────────────────────────────────────────────────────

#[test]
fn admin_help_prints_usage() {
    assert_help_ok(&["admin"]);
}

#[test]
fn admin_get_help_prints_usage() {
    assert_help_ok(&["admin", "get"]);
}

#[test]
fn admin_propose_help_prints_usage() {
    assert_help_ok(&["admin", "propose"]);
}

#[test]
fn admin_accept_help_prints_usage() {
    assert_help_ok(&["admin", "accept"]);
}

// ── ops ───────────────────────────────────────────────────────────────────────

#[test]
fn ops_help_prints_usage() {
    assert_help_ok(&["ops"]);
}

#[test]
fn ops_pause_help_prints_usage() {
    assert_help_ok(&["ops", "pause"]);
}

#[test]
fn ops_unpause_help_prints_usage() {
    assert_help_ok(&["ops", "unpause"]);
}

#[test]
fn ops_is_paused_help_prints_usage() {
    assert_help_ok(&["ops", "is-paused"]);
}

// ── deploy ────────────────────────────────────────────────────────────────────

#[test]
fn deploy_help_prints_usage() {
    assert_help_ok(&["deploy"]);
}
