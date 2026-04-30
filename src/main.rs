use clap::{Parser, Subcommand};
use color_eyre::Result;

use pilegit::{core, git, tui};

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
    /// Launch interactive TUI (default when no subcommand given)
    Tui,
    /// Initialize pilegit config for this repository
    Init,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Tui) => {
            ensure_config()?;
            tui::run()
        }
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Init) => cmd_init(),
    }
}

/// Ensure config exists, run setup wizard if not.
fn ensure_config() -> Result<()> {
    let repo = git::ops::Repo::open()?;
    if core::config::Config::load(&repo.workdir).is_none() {
        println!("  No .pilegit.toml found. Running setup...");
        core::config::run_setup(&repo.workdir)?;
    }
    Ok(())
}

fn cmd_init() -> Result<()> {
    let repo = git::ops::Repo::open()?;
    let config = core::config::run_setup(&repo.workdir)?;
    core::config::check_dependencies(&config);
    Ok(())
}

fn cmd_status() -> Result<()> {
    let repo = git::repo_loader::open_resolved()?;
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
