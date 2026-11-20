//! Developer tasks for Lafiya-contract. Run with `cargo xtask <task>`.

use clap::{Parser, Subcommand};

mod docs;
mod sandbox;

#[derive(Parser, Debug)]
#[command(name = "cargo xtask", about = "Lafiya developer tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand, Debug)]
enum Task {
    /// Generate the contract and CLI reference pages of the docs site
    Docs {
        /// Fail instead of writing if the committed pages are out of date
        #[arg(long)]
        check: bool,
    },
    /// Local sandbox chain seeded from a scenario file (DEV ONLY keys)
    Sandbox {
        #[command(subcommand)]
        sub: sandbox::SandboxSub,
    },
    /// Testnet rehearsal of the incentive-pool payout flow against real testnet USDC
    Rehearse {
        /// Emit the machine-readable report to this path (defaults to stdout)
        #[arg(long)]
        report: Option<std::path::PathBuf>,
        /// Fail the run if any rehearsal step does not match its expected outcome
        #[arg(long)]
        strict: bool,
    },
    /// Build all workspace members (including lafiya-indexer and lafiya-qr)
    Build,
    /// Lint all workspace members with clippy, denying warnings
    Lint,
    /// Test all workspace members (including lafiya-indexer and lafiya-qr)
    Test,
    /// Run build, lint, and test across all workspace members
    Check,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().task {
        Task::Docs { check } => docs::run(check),
        Task::Sandbox { sub } => sandbox::run(&sub),
        Task::Rehearse { report, strict } => rehearse(report.as_deref(), strict),
        Task::Build => build(),
        Task::Lint => lint(),
        Task::Test => test(),
        Task::Check => {
            build()?;
            lint()?;
            test()
        }
    }
}

/// Run a cargo command over the whole workspace, streaming its output.
fn cargo(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new(env!("CARGO"))
        .args(args)
        .status()?;
    if !status.success() {
        anyhow::bail!("cargo {} failed with {status}", args.join(" "));
    }
    Ok(())
}

fn build() -> anyhow::Result<()> {
    cargo(&["build", "--workspace", "--all-targets"])
}

fn lint() -> anyhow::Result<()> {
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])
}

fn test() -> anyhow::Result<()> {
    cargo(&["test", "--workspace", "--all-features"])
}

/// Invoke the scripted testnet rehearsal and forward its machine-readable report.
fn rehearse(report: Option<&std::path::Path>, strict: bool) -> anyhow::Result<()> {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("rehearse_incentive_pool.sh");
    if !script.exists() {
        anyhow::bail!("rehearsal script not found at {}", script.display());
    }

    let mut cmd = std::process::Command::new("bash");
    cmd.arg(&script);
    if let Some(path) = report {
        cmd.arg("--report").arg(path);
    }
    if strict {
        cmd.arg("--strict");
    }

    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("testnet rehearsal failed with {status}");
    }
    Ok(())
}