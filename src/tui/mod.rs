pub mod app;
pub mod input;
pub mod ui;

use std::io::{self, Write};
use std::process::Command;

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::core::stack::Stack;
use crate::git::ops::Repo;
use app::{App, SuspendReason};

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Launch the interactive TUI with suspend/resume support.
pub fn run() -> Result<()> {
    let repo = Repo::open()?;
    let base = repo.detect_base()?;
    let commits = repo.list_stack_commits()?;
    let stack = Stack::new(base, commits);
    let mut app = App::new(stack);

    loop {
        // Initialize terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Restore terminal on panic
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            original_hook(panic);
        }));

        // Run the TUI until quit or suspend
        app.run(&mut terminal)?;

        // Restore terminal
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

        if app.should_quit {
            break;
        }

        // Handle the suspend reason
        match app.wants_suspend.take() {
            Some(SuspendReason::InsertAtHead) => handle_insert_at_head(&mut app)?,
            Some(SuspendReason::InsertAfterCursor { hash }) => {
                handle_insert_after(&mut app, &hash)?;
            }
            Some(SuspendReason::EditCommit { hash }) => handle_edit_commit(&mut app, &hash)?,
            Some(SuspendReason::SquashCommits { hashes, default_subject, default_body }) => {
                handle_squash_commits(&mut app, &hashes, &default_subject, &default_body)?;
            }
            Some(SuspendReason::RebaseConflict) => handle_rebase_conflict(&mut app)?,
            None => break,
        }
    }

    Ok(())
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

fn print_box(color: &str, title: &str, lines: &[&str]) {
    let bar = "─".repeat(55);
    println!("\n\x1b[1;{color}m┌─ {title} {bar}\x1b[0m");
    for line in lines {
        println!("\x1b[1;{color}m│\x1b[0m  {line}");
    }
    println!("\x1b[1;{color}m└{bar}──\x1b[0m\n");
}

fn wait_for_enter() -> Result<()> {
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(())
}

fn get_editor() -> String {
    std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "nano".to_string())
}

/// Read a single keypress using crossterm (no need to press Enter).
fn read_single_key() -> Result<char> {
    enable_raw_mode()?;
    let ch = loop {
        if let Event::Key(key) = event::read()? {
            if let KeyCode::Char(c) = key.code {
                break c;
            }
        }
    };
    disable_raw_mode()?;
    Ok(ch)
}

// -------------------------------------------------------------------
// Suspend handlers
// -------------------------------------------------------------------

/// Insert a new commit at the top of the stack (HEAD).
fn handle_insert_at_head(app: &mut App) -> Result<()> {
    print_box("36", "pilegit: insert commit at top", &[
        "Make your changes and commit:",
        "",
        "  \x1b[1;33mgit add <files> && git commit -m \"...\"\x1b[0m",
        "",
        "Press \x1b[1;32mEnter\x1b[0m when done to return.",
    ]);
    wait_for_enter()?;
    match app.reload_stack() {
        Ok(()) => {
            app.record_reload("insert commit at top");
            app.notify("Stack refreshed.");
        }
        Err(e) => app.notify(format!("Reload failed: {}", e)),
    }
    Ok(())
}

/// Insert a new commit after the cursor position using rebase --break.
fn handle_insert_after(app: &mut App, hash: &str) -> Result<()> {
    let repo = Repo::open()?;
    match repo.rebase_break_after(hash) {
        Ok(false) => {
            // Rebase paused at the break point — user can now commit
            print_box("36", &format!("pilegit: insert after {}", hash), &[
                "Rebase paused at the right position.",
                "Make your changes and commit:",
                "",
                "  \x1b[1;33mgit add <files> && git commit -m \"...\"\x1b[0m",
                "",
                "Press \x1b[1;32mEnter\x1b[0m when done. pilegit will rebase the rest.",
            ]);
            wait_for_enter()?;

            // Continue the rebase to replay remaining commits on top
            match repo.rebase_continue() {
                Ok(true) => {
                    app.reload_stack()?;
                    app.record_reload("insert commit after cursor");
                    app.notify("Inserted commit and rebased.");
                }
                Ok(false) => {
                    app.notify("Conflict during rebase after insert.");
                    app.wants_suspend = Some(SuspendReason::RebaseConflict);
                }
                Err(e) => app.notify(format!("Rebase continue failed: {}", e)),
            }
        }
        Ok(true) => {
            // Rebase completed without breaking — commit wasn't found or at top
            app.notify("Break point not reached. Try inserting at top instead (t).");
        }
        Err(e) => app.notify(format!("Insert failed: {}", e)),
    }
    Ok(())
}

/// Edit/amend a specific commit.
/// Pauses rebase at the commit, lets the user make changes, then
/// auto-stages + amends when the user presses Enter.
fn handle_edit_commit(app: &mut App, hash: &str) -> Result<()> {
    let repo = Repo::open()?;
    match repo.rebase_edit_commit(hash) {
        Ok(false) => {
            // Rebase paused at the target commit — user can now edit
            print_box("33", &format!("pilegit: editing commit {}", hash), &[
                "Rebase is paused at this commit.",
                "The working tree has the state of this commit.",
                "Make your changes to the code now.",
                "",
                "When you press \x1b[1;32mEnter\x1b[0m, pilegit will:",
                "  1. Stage all changes  (git add -A)",
                "  2. Amend this commit  (git commit --amend --no-edit)",
                "  3. Rebase the remaining commits on top",
                "",
                "Press \x1b[1;32mEnter\x1b[0m when ready.",
            ]);
            wait_for_enter()?;

            // Auto-stage and amend
            println!("  Staging and amending...");
            let _ = Command::new("git")
                .current_dir(&repo.workdir)
                .args(["add", "-A"])
                .output();
            let amend = Command::new("git")
                .current_dir(&repo.workdir)
                .args(["commit", "--amend", "--no-edit"])
                .output()?;

            if !amend.status.success() {
                let stderr = String::from_utf8_lossy(&amend.stderr);
                if !stderr.contains("nothing to commit") {
                    app.notify(format!("Amend warning: {}", stderr.trim()));
                }
            }

            // Continue the rebase to replay remaining commits
            match repo.rebase_continue() {
                Ok(true) => {
                    app.reload_stack()?;
                    app.record_reload(&format!("edit commit {}", hash));
                    app.notify("Commit edited and rebased successfully.");
                }
                Ok(false) => {
                    app.notify("Conflict during rebase after edit.");
                    app.wants_suspend = Some(SuspendReason::RebaseConflict);
                }
                Err(e) => app.notify(format!("Rebase continue failed: {}", e)),
            }
        }
        Ok(true) => {
            app.notify("Commit not found in stack range.");
        }
        Err(e) => app.notify(format!("Edit failed: {}", e)),
    }
    Ok(())
}

/// Squash commits: open editor for message, then perform actual git squash.
fn handle_squash_commits(
    app: &mut App,
    hashes: &[String],
    default_subject: &str,
    default_body: &str,
) -> Result<()> {
    // Build initial message content for the editor
    let initial_content = if default_body.is_empty() {
        format!("{}\n", default_subject)
    } else {
        format!("{}\n\n{}\n", default_subject, default_body)
    };

    // Write to temp file
    let tmp_path = std::env::temp_dir().join(format!("pgit-squash-msg-{}.txt", std::process::id()));
    std::fs::write(&tmp_path, &initial_content)?;

    let editor = get_editor();
    println!();
    print_box("36", "pilegit: edit squash message", &[
        &format!("Squashing {} commits. Edit the combined commit message.", hashes.len()),
        "",
        &format!("  Editor: \x1b[1;33m{}\x1b[0m", editor),
        "",
        "  First line = commit subject",
        "  Remaining lines = commit body",
        "",
        "  Save and close the editor when done.",
    ]);

    let status = Command::new(&editor)
        .arg(&tmp_path)
        .status();

    match status {
        Ok(s) if s.success() => {
            let edited = std::fs::read_to_string(&tmp_path)?;
            let _ = std::fs::remove_file(&tmp_path);

            let message = edited.trim().to_string();
            if message.is_empty() {
                app.notify("Empty message — squash cancelled.");
                return Ok(());
            }

            // Now perform the actual git squash
            println!("  Squashing in git...");
            let repo = Repo::open()?;
            match repo.squash_commits_with_message(hashes, &message) {
                Ok(true) => {
                    app.reload_stack()?;
                    app.record_reload(&format!("squash {} commits", hashes.len()));
                    app.notify(format!("Squashed {} commits.", hashes.len()));
                }
                Ok(false) => {
                    app.notify("Conflict during squash.");
                    app.wants_suspend = Some(SuspendReason::RebaseConflict);
                }
                Err(e) => app.notify(format!("Squash failed: {}", e)),
            }
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp_path);
            app.notify("Editor exited with error — squash cancelled.");
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            app.notify(format!("Could not open {}: {}", editor, e));
        }
    }

    Ok(())
}

/// Handle rebase conflicts — single-keypress resolution loop.
fn handle_rebase_conflict(app: &mut App) -> Result<()> {
    loop {
        let repo = Repo::open()?;
        let conflicts = repo.conflicted_files().unwrap_or_default();

        let mut lines: Vec<String> = vec![];
        if conflicts.is_empty() {
            lines.push("No remaining conflicts detected.".into());
        } else {
            lines.push("Conflicting files:".into());
            for f in &conflicts {
                lines.push(format!("  \x1b[1;33m{}\x1b[0m", f));
            }
        }
        lines.extend_from_slice(&[
            String::new(),
            "Resolve conflicts, then stage: \x1b[1;33mgit add <files>\x1b[0m".into(),
            String::new(),
            "Press \x1b[1;32mc\x1b[0m to continue  or  \x1b[1;31ma\x1b[0m to abort".into(),
        ]);

        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        print_box("31", "pilegit: rebase conflict", &line_refs);

        // Single keypress — no need to press Enter
        let choice = read_single_key()?;

        match choice {
            'c' => {
                println!("  Continuing rebase...");
                match app.continue_rebase() {
                    Ok(true) => return Ok(()),
                    Ok(false) => continue, // more conflicts
                    Err(e) => {
                        app.notify(format!("Continue failed: {}", e));
                        return Ok(());
                    }
                }
            }
            'a' => {
                println!("  Aborting rebase...");
                let _ = app.abort_rebase();
                return Ok(());
            }
            _ => {
                println!("  Press 'c' to continue or 'a' to abort.");
            }
        }
    }
}
