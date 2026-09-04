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

#[test]
fn top_level_help_prints_usage() {
    assert_help_ok(&[]);
}

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
fn attester_remove_help_prints_usage() {
    assert_help_ok(&["attester", "remove"]);
}

#[test]
fn attestation_help_prints_usage() {
    assert_help_ok(&["attestation"]);
}

#[test]
fn attestation_get_help_prints_usage() {
    assert_help_ok(&["attestation", "get"]);
}

#[test]
fn deploy_help_prints_usage() {
    assert_help_ok(&["deploy"]);
}
