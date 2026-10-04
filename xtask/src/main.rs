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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().task {
        Task::Docs { check } => docs::run(check),
        Task::Sandbox { sub } => sandbox::run(&sub),
    }
}
