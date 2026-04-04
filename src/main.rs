use clap::{Parser, Subcommand};
use color_eyre::Result;

mod core;
mod git;
mod tui;

#[derive(Parser)]
#[command(
    name = "pgit",
    about = "pilegit — git stacking with style",
    version,
    after_help = "Run `pgit` with no arguments to launch the interactive TUI."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show the current stack (non-interactive)
    Status,
    /// Submit PRs for each commit in the stack
    Submit,
    /// Sync stack with remote (pull + rebase)
    Sync,
    /// Launch interactive TUI (default when no subcommand given)
    Tui,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Tui) => tui::run(),
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Submit) => {
            println!("PR submission not yet implemented — coming soon.");
            Ok(())
        }
        Some(Commands::Sync) => {
            println!("Sync not yet implemented — coming soon.");
            Ok(())
        }
    }
}

fn cmd_status() -> Result<()> {
    let repo = git::ops::Repo::open()?;
    let commits = repo.list_stack_commits()?;
    if commits.is_empty() {
        println!("No commits ahead of base branch.");
    } else {
        println!("pilegit stack ({} commits):\n", commits.len());
        for (i, c) in commits.iter().enumerate() {
            let marker = if i == 0 { "→" } else { " " };
            let hash_short = &c.hash[..c.hash.len().min(8)];
            println!("  {} {} {}", marker, hash_short, c.subject);
        }
    }
    Ok(())
}
