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