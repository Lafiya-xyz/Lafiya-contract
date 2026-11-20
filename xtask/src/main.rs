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
    /// Build all workspace members (including lafiya-indexer and lafiya-qr)
    Build,
    /// Lint all workspace members with clippy, denying warnings
    Lint,
    /// Test all workspace members (including lafiya-indexer and lafiya-qr)
    Test,
    /// Run build, lint, and test across all workspace members
    Check,
    /// Build the target wasm and report instance-storage entry size per contract
    Footprint {
        /// Scenario file used to seed the sandbox before measuring
        #[arg(long, default_value = "scenarios/seed.toml")]
        scenario: std::path::PathBuf,
    },
    /// Run the interface and storage-layout conformance checks
    Conformance {
        /// Baseline wasm (or release manifest) to compare the storage layout against
        #[arg(long)]
        baseline: Option<std::path::PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().task {
        Task::Docs { check } => docs::run(check),
        Task::Sandbox { sub } => sandbox::run(&sub),
        Task::Build => build(),
        Task::Lint => lint(),
        Task::Test => test(),
        Task::Check => {
            build()?;
            lint()?;
            test()
        }
        Task::Footprint { scenario } => footprint(&scenario),
        Task::Conformance { baseline } => conformance(baseline.as_deref()),
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

/// Build the target wasm, seed the sandbox from `scenario`, and report the
/// instance-storage entry size of every contract so cold-key relocation can be
/// measured before and after.
fn footprint(scenario: &std::path::Path) -> anyhow::Result<()> {
    cargo(&[
        "build",
        "--workspace",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ])?;
    let status = std::process::Command::new(env!("CARGO"))
        .args(["run", "-p", "xtask", "--", "sandbox", "seed", "--scenario"])
        .arg(scenario)
        .status()?;
    if !status.success() {
        anyhow::bail!("sandbox seeding failed with {status}");
    }
    let script = std::path::Path::new("scripts/conformance/instance_footprint.py");
    let status = std::process::Command::new("python3")
        .arg(script)
        .arg("--report")
        .status()?;
    if !status.success() {
        anyhow::bail!("instance footprint report failed with {status}");
    }
    Ok(())
}

/// Run the interface conformance check (#711) and the static storage-layout
/// compatibility check (#714) against the given baseline.
fn conformance(baseline: Option<&std::path::Path>) -> anyhow::Result<()> {
    let interface = std::path::Path::new("scripts/conformance/extract_interface.py");
    let status = std::process::Command::new("python3")
        .arg(interface)
        .arg("--check")
        .status()?;
    if !status.success() {
        anyhow::bail!("interface conformance check failed with {status}");
    }
    let layout = std::path::Path::new("scripts/conformance/storage_layout.py");
    let mut cmd = std::process::Command::new("python3");
    cmd.arg(layout).arg("--check");
    if let Some(baseline) = baseline {
        cmd.arg("--baseline").arg(baseline);
    }
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("storage-layout compatibility check failed with {status}");
    }
    Ok(())
}