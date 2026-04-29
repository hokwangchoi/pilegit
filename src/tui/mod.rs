pub mod app;
pub mod input;
pub mod ui;

use std::io::{self, Write};
use std::process::Command;

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::core::config::Config;
use crate::core::stack::Stack;
use crate::forge;
use crate::git::ops::sed_inplace_shell_prefix;
use crate::git::repo_loader;
use app::{App, SuspendReason};

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Launch the interactive TUI with suspend/resume support.
pub fn run() -> Result<()> {
    let repo = repo_loader::open_resolved()?;

    // Block startup if a rebase is in progress from a previous session
    if repo.is_rebase_in_progress() {
        eprintln!("  \x1b[31m⚠ A rebase is already in progress.\x1b[0m");
        eprintln!(
            "  Run \x1b[1mgit rebase --continue\x1b[0m or \x1b[1mgit rebase --abort\x1b[0m first."
        );
        return Ok(());
    }

    // Block startup if there are uncommitted changes (staged or unstaged)
    if repo.has_uncommitted_changes() {
        eprintln!("  \x1b[31m⚠ You have uncommitted changes.\x1b[0m");
        eprintln!("  Commit or stash them before running pgit.");
        return Ok(());
    }

    let base = repo.base()?;
    let mut commits = repo.list_stack_commits()?;

    // Load config and create the forge integration
    let config = Config::load_or_default(&repo.workdir);
    let f = forge::create_forge(&config);

    // Check that required CLI tools are installed
    crate::core::config::check_dependencies(&config);

    // Mark submitted status via the forge
    f.mark_submitted(&repo, &mut commits);

    let stack = Stack::new(base, commits);
    let mut app = App::new(stack, f);

    // Restore terminal on panic — set once, not per loop iteration
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    loop {
        // Initialize terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

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
            Some(SuspendReason::SquashCommits {
                hashes,
                default_body,
                trailers,
            }) => {
                handle_squash_commits(&mut app, &hashes, &default_body, &trailers)?;
            }
            Some(SuspendReason::SubmitCommit {
                hash,
                subject,
                cursor_index,
            }) => {
                handle_submit_commit(&mut app, &hash, &subject, cursor_index)?;
            }
            Some(SuspendReason::UpdatePR {
                hash,
                subject,
                cursor_index,
            }) => {
                handle_update_pr(&mut app, &hash, &subject, cursor_index)?;
            }
            Some(SuspendReason::SyncPRs) => {
                handle_sync_prs(&mut app)?;
            }
            Some(SuspendReason::PullRemote) => {
                handle_pull_remote(&mut app)?;
            }
            Some(SuspendReason::Rebase) => {
                handle_rebase(&mut app)?;
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
    let width: usize = 62;
    // Top: ┌─ title ───...───┐
    let title_part = format!("─ {} ", title);
    let title_visible = strip_ansi_len(&title_part);
    let top_fill = "─".repeat(width.saturating_sub(2 + title_visible));
    println!("\n\x1b[1;{color}m┌{title_part}{top_fill}┐\x1b[0m");
    // Middle: │  content     │
    let inner = width.saturating_sub(5); // 1(│) + 2(spaces) + content + pad + 1(space) + 1(│)
    for line in lines {
        let visible_len = strip_ansi_len(line);
        let pad = inner.saturating_sub(visible_len);
        println!(
            "\x1b[1;{color}m│\x1b[0m  {line}{} \x1b[1;{color}m│\x1b[0m",
            " ".repeat(pad)
        );
    }
    // Bottom: └───...───┘
    let bottom = "─".repeat(width.saturating_sub(2));
    println!("\x1b[1;{color}m└{bottom}┘\x1b[0m\n");
}

/// Calculate visible string length ignoring ANSI escape codes.
fn strip_ansi_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

fn wait_for_enter() -> Result<()> {
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(())
}

/// Clear the terminal screen for a fresh operation display.
fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = io::stdout().flush();
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
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                if let KeyCode::Char(c) = key.code {
                    break c;
                }
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
    print_box(
        "36",
        "pilegit: insert commit at top",
        &[
            "Make your changes and commit:",
            "",
            "  \x1b[1;33mgit add <files> && git commit -m \"...\"\x1b[0m",
            "",
            "Press \x1b[1;32mEnter\x1b[0m when done to return.",
        ],
    );
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
    let repo = repo_loader::open_resolved()?;
    match repo.rebase_break_after(hash) {
        Ok(false) => {
            // Rebase paused at the break point — user can now commit
            print_box(
                "36",
                &format!("pilegit: insert after {}", hash),
                &[
                    "Rebase paused at the right position.",
                    "Make your changes and commit:",
                    "",
                    "  \x1b[1;33mgit add <files> && git commit -m \"...\"\x1b[0m",
                    "",
                    "Press \x1b[1;32mEnter\x1b[0m when done. pilegit will rebase the rest.",
                ],
            );
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
    let repo = repo_loader::open_resolved()?;
    match repo.rebase_edit_commit(hash) {
        Ok(false) => {
            // Rebase paused at the target commit — user can now edit
            print_box(
                "33",
                &format!("pilegit: editing commit {}", hash),
                &[
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
                ],
            );
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
    default_body: &str,
    trailers: &[String],
) -> Result<()> {
    // Build editor content: commit messages, then any existing trailers
    let mut initial_content = format!(
        "# Pick or combine the commit messages below.\n\
         # The first line becomes the commit subject.\n\
         # Lines starting with # are ignored.\n\n\
         {}\n",
        default_body
    );
    if !trailers.is_empty() {
        initial_content.push('\n');
        for t in trailers {
            initial_content.push_str(t);
            initial_content.push('\n');
        }
    }

    // Write to temp file
    let tmp_path = std::env::temp_dir().join(format!("pgit-squash-msg-{}.txt", std::process::id()));
    std::fs::write(&tmp_path, &initial_content)?;

    let editor = get_editor();
    println!();
    print_box(
        "36",
        "pilegit: edit squash message",
        &[
            &format!(
                "Squashing {} commits. Edit the combined commit message.",
                hashes.len()
            ),
            "",
            &format!("  Editor: \x1b[1;33m{}\x1b[0m", editor),
            "",
            "  First line = commit subject",
            "  Remaining lines = commit body",
            "",
            "  Save and close the editor when done.",
        ],
    );

    let status = Command::new(&editor).arg(&tmp_path).status();

    match status {
        Ok(s) if s.success() => {
            let edited = std::fs::read_to_string(&tmp_path)?;
            let _ = std::fs::remove_file(&tmp_path);

            // Strip comment lines (starting with #) and trim
            let message: String = edited
                .lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");
            let mut message = message.trim().to_string();
            if message.is_empty() {
                app.notify("Empty message — squash cancelled.");
                return Ok(());
            }

            // Re-append any trailers that were removed during editing
            for trailer in trailers {
                if !message.contains(trailer.as_str()) {
                    message.push_str("\n\n");
                    message.push_str(trailer);
                }
            }

            // Now perform the actual git squash
            clear_screen();
            println!();
            println!("  \x1b[1;36m▸ Squashing {} commits...\x1b[0m", hashes.len());
            println!();
            let repo = repo_loader::open_resolved()?;
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

/// Submit a commit as a PR: open editor for description, then submit.
fn handle_submit_commit(
    app: &mut App,
    hash: &str,
    subject: &str,
    cursor_index: usize,
) -> Result<()> {
    clear_screen();
    let short = &hash[..7.min(hash.len())];

    let repo = repo_loader::open_resolved()?;

    // Safety check: if a remote branch exists and differs from what pgit
    // last pushed (saved in state file), block to prevent overwriting
    let _ = repo.fetch_origin();
    let branch_name = repo.make_pgit_branch_name(subject);
    let remote = format!("origin/{}", branch_name);
    let remote_hash = repo
        .git_pub(&["rev-parse", &remote])
        .ok()
        .map(|h| h.trim().to_string());
    let saved = repo.read_sync_state();
    let saved_hash = saved.get(&branch_name).cloned();
    if let (Some(rh), Some(sh)) = (&remote_hash, &saved_hash) {
        if rh != sh {
            println!();
            println!(
                "  \x1b[1;31m⚠ Remote branch has newer changes that would be overwritten.\x1b[0m"
            );
            println!();
            println!("  Press \x1b[1;32mP\x1b[0m to pull remote changes first.");
            println!();
            println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
            wait_for_enter()?;
            app.notify("Submit aborted — remote has newer changes. Press P to pull.");
            return Ok(());
        }
    }

    let (open_prs, gh_avail) = app.forge.list_open(&repo);
    let base =
        repo.determine_base_for_commit(&app.stack.patches, cursor_index, &open_prs, gh_avail);

    if !app.forge.needs_description_editor() {
        // Platform has its own editor (e.g. arc diff) — run directly
        clear_screen();
        println!();
        println!("  \x1b[1;36m▸ Submitting: {}\x1b[0m", subject);
        println!();
        match app.forge.submit(&repo, hash, subject, &base, "") {
            Ok(out) => {
                println!();
                println!("  \x1b[32m✓ {}\x1b[0m", out);
                let _ = app.reload_stack();
                app.forge.save_sync_state(&repo, &app.stack.patches);
                app.notify(out);
            }
            Err(e) => {
                println!();
                println!("  \x1b[31m✗ Submit failed: {}\x1b[0m", e);
                app.notify(format!("Submit failed: {}", e));
            }
        }
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        return Ok(());
    }

    // Open pilegit's editor for PR description
    let config = Config::load_or_default(&repo.workdir);
    let template = forge::pr_description::compose_initial_draft(
        &repo,
        app.forge.as_ref(),
        &config.repo,
        subject,
    );

    let tmp_path = std::env::temp_dir().join(format!("pgit-pr-msg-{}.txt", std::process::id()));
    std::fs::write(&tmp_path, &template)?;

    let editor = get_editor();
    print_box(
        "36",
        &format!("pilegit: submit {}", short),
        &[
            "Write your PR description.",
            "",
            &format!("  Editor: \x1b[1;33m{}\x1b[0m", editor),
            &format!("  Commit: \x1b[1;33m{} {}\x1b[0m", short, subject),
            &format!("  Base:   \x1b[1;33m{}\x1b[0m", base),
            "",
            "  Save and close the editor when done.",
            "  Leave empty to cancel.",
        ],
    );

    let status = Command::new(&editor).arg(&tmp_path).status();

    match status {
        Ok(s) if s.success() => {
            let body = std::fs::read_to_string(&tmp_path)?;
            let _ = std::fs::remove_file(&tmp_path);
            let body = body.trim().to_string();

            if body.is_empty() {
                app.notify("Empty description — submit cancelled.");
                return Ok(());
            }

            clear_screen();
            println!();
            println!("  \x1b[1;36m▸ Submitting: {}\x1b[0m", subject);
            println!();
            println!("    \x1b[33mPushing branch and creating PR...\x1b[0m");

            match app.forge.submit(&repo, hash, subject, &base, &body) {
                Ok(out) => {
                    println!();
                    println!("  \x1b[32m✓ {}\x1b[0m", out);
                    let _ = app.reload_stack();
                    app.forge.save_sync_state(&repo, &app.stack.patches);
                    app.notify(out);
                }
                Err(e) => {
                    println!();
                    println!("  \x1b[31m✗ Submit failed: {}\x1b[0m", e);
                    app.notify(format!("Submit failed: {}", e));
                }
            }

            println!();
            println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
            wait_for_enter()?;
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp_path);
            app.notify("Editor exited with error — submit cancelled.");
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            app.notify(format!("Could not open {}: {}", editor, e));
        }
    }

    Ok(())
}

/// Update an existing PR with progress display.
fn handle_update_pr(app: &mut App, hash: &str, subject: &str, cursor_index: usize) -> Result<()> {
    clear_screen();
    let short = &hash[..7.min(hash.len())];

    let repo = repo_loader::open_resolved()?;

    // Safety check: abort if remote has changes we'd overwrite
    let patches = app.stack.patches.clone();
    let diverged = app.forge.check_diverged(&repo, &patches);
    let branch_name = repo.make_pgit_branch_name(subject);
    let this_diverged = diverged.iter().any(|(b, _)| {
        b == &branch_name
            || patches
                .get(cursor_index)
                .and_then(|p| p.pr_number.map(|n| format!("D{}", n)))
                .map(|key| b == &key)
                .unwrap_or(false)
    });

    if this_diverged {
        println!();
        println!(
            "  \x1b[1;31m⚠ Remote has newer changes for this PR that would be overwritten.\x1b[0m"
        );
        println!();
        println!("  Press \x1b[1;32mP\x1b[0m to pull remote changes first, then update.");
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        app.notify("Update aborted — remote has newer changes. Press P to pull.");
        return Ok(());
    }

    println!();
    println!("  \x1b[1;36m▸ Updating PR: {}\x1b[0m", subject);
    println!();

    println!("    \x1b[33mDetermining base...\x1b[0m");
    let (open_prs, gh_avail) = app.forge.list_open(&repo);
    let pr_base =
        repo.determine_base_for_commit(&app.stack.patches, cursor_index, &open_prs, gh_avail);

    clear_screen();
    println!();
    println!("  \x1b[1;36m▸ Updating PR: {}\x1b[0m", subject);
    println!();
    println!("    \x1b[33mForce-pushing {} → {}\x1b[0m", short, pr_base);

    match app.forge.update(&repo, hash, subject, &pr_base) {
        Ok(msg) => {
            println!();
            println!("  \x1b[32m✓ {}\x1b[0m", msg);
            let _ = app.reload_stack();
            app.forge.save_sync_state(&repo, &app.stack.patches);
            app.notify(msg);
        }
        Err(e) => {
            println!();
            println!("  \x1b[31m✗ Update failed: {}\x1b[0m", e);
            app.notify(format!("Update failed: {}", e));
        }
    }

    println!();
    println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
    wait_for_enter()?;
    Ok(())
}

/// Rebase onto base with progress display. After rebasing, syncs any
/// submitted PRs to update their branches with the new commit hashes.
fn handle_rebase(app: &mut App) -> Result<()> {
    clear_screen();
    println!();
    println!("  \x1b[1;36m▸ Rebasing stack...\x1b[0m");
    println!();

    let repo = repo_loader::open_resolved()?;
    match repo.rebase_onto_base(&|msg| {
        println!("    \x1b[33m{}\x1b[0m", msg);
    }) {
        Ok(true) => {
            app.reload_stack()?;
            app.record_reload("rebase onto base");

            // Fix dependency trailers (e.g. "Depends on DXXX" for Phabricator)
            if let Ok(r) = repo_loader::open_resolved() {
                let _ = app.forge.fix_dependencies(&r);
                let _ = app.reload_stack();
            }

            clear_screen();
            println!();
            println!(
                "  \x1b[32m✓ Rebase completed. Stack: {} commits.\x1b[0m",
                app.stack.len()
            );

            // Sync submitted PRs if any
            let submitted_count = app
                .stack
                .patches
                .iter()
                .filter(|p| p.status == crate::core::stack::PatchStatus::Submitted)
                .count();
            if submitted_count > 0 {
                // Safety check: abort sync if remote has newer changes
                if let Ok(r) = repo_loader::open_resolved() {
                    let patches = app.stack.patches.clone();
                    let diverged = app.forge.check_diverged(&r, &patches);
                    if !diverged.is_empty() {
                        println!();
                        println!("  \x1b[1;31m⚠ Remote has newer changes — skipping sync.\x1b[0m");
                        for (_, desc) in &diverged {
                            println!("    \x1b[33m{}\x1b[0m", desc);
                        }
                        println!();
                        println!("  Press \x1b[1;32mP\x1b[0m to pull remote changes, then sync.");
                        let _ = app.reload_stack();
                        prompt_cleanup_stale_branches(app)?;
                        app.notify("Rebase done, sync skipped — remote has newer changes. Press P to pull.");
                        println!();
                        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
                        wait_for_enter()?;
                        return Ok(());
                    }
                }

                println!();
                println!(
                    "  \x1b[1;36m▸ Syncing {} submitted PRs...\x1b[0m",
                    submitted_count
                );
                println!();
                if let Ok(r) = repo_loader::open_resolved() {
                    let patches = app.stack.patches.clone();
                    match app.forge.sync(&r, &patches, &|msg| {
                        println!("    \x1b[33m{}\x1b[0m", msg);
                    }) {
                        Ok(updates) => {
                            clear_screen();
                            println!();
                            println!(
                                "  \x1b[32m✓ Rebase completed. Stack: {} commits.\x1b[0m",
                                app.stack.len()
                            );
                            println!();
                            println!("  \x1b[32m✓ Synced {} PRs:\x1b[0m", updates.len());
                            for u in &updates {
                                println!("    {}", u);
                            }
                            app.forge.save_sync_state(&r, &app.stack.patches);
                        }
                        Err(e) => println!("    \x1b[31mSync warning: {}\x1b[0m", e),
                    }
                }
                let _ = app.reload_stack();
            }

            // Check for stale branches
            prompt_cleanup_stale_branches(app)?;

            app.notify("Rebase completed.");
        }
        Ok(false) => {
            clear_screen();
            println!();
            println!("  \x1b[31m⚠ Conflicts detected.\x1b[0m");
            app.notify("Conflict during rebase.");
            app.wants_suspend = Some(SuspendReason::RebaseConflict);
            return Ok(());
        }
        Err(e) => {
            clear_screen();
            println!();
            println!("  \x1b[31m✗ Rebase failed: {}\x1b[0m", e);
            app.notify(format!("Rebase failed: {}", e));
        }
    }

    println!();
    println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
    wait_for_enter()?;
    Ok(())
}

/// Sync all submitted PRs with progress display.
fn handle_sync_prs(app: &mut App) -> Result<()> {
    clear_screen();
    println!();
    println!("  \x1b[1;36m▸ Syncing PRs...\x1b[0m");
    println!();

    let repo = repo_loader::open_resolved()?;

    // Safety check: abort if remote has changes we'd overwrite
    let patches = app.stack.patches.clone();
    let diverged = app.forge.check_diverged(&repo, &patches);
    if !diverged.is_empty() {
        clear_screen();
        println!();
        println!("  \x1b[1;31m⚠ Remote has newer changes that would be overwritten:\x1b[0m");
        println!();
        for (_, desc) in &diverged {
            println!("    \x1b[33m{}\x1b[0m", desc);
        }
        println!();
        println!("  Press \x1b[1;32mP\x1b[0m to pull remote changes first, then sync.");
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        app.notify("Sync aborted — remote has newer changes. Press P to pull.");
        return Ok(());
    }

    // Fix dependency trailers before syncing (e.g. "Depends on DXXX" for Phabricator)
    let _ = app.forge.fix_dependencies(&repo);
    let _ = app.reload_stack();

    let repo = repo_loader::open_resolved()?;
    let base = repo.base()?;
    let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();
    let patches = app.stack.patches.clone();

    match app.forge.sync(&repo, &patches, &|msg| {
        println!("    \x1b[33m{}\x1b[0m", msg);
    }) {
        Ok(updates) => {
            clear_screen();
            println!();
            if updates.is_empty() {
                println!("  \x1b[32m✓ No open PRs to sync.\x1b[0m");
            } else {
                println!("  \x1b[32m✓ Synced {} PRs:\x1b[0m", updates.len());
                println!();
                for u in &updates {
                    println!("    {}", u);
                }

                // Find PRs ready to merge
                let ready: Vec<&String> = updates
                    .iter()
                    .filter(|u| u.starts_with("✓") && u.ends_with(&format!("→ {}", base_branch)))
                    .collect();
                if !ready.is_empty() {
                    println!();
                    println!("  \x1b[1;32m▸ Ready to merge into {}:\x1b[0m", base_branch);
                    for r in &ready {
                        let branch = r.trim_start_matches("✓ ").split(" → ").next().unwrap_or(r);
                        println!("    \x1b[1;33m{}\x1b[0m", branch);
                    }
                }

                // Warn about failures
                let failed: Vec<&String> = updates.iter().filter(|u| u.starts_with("⚠")).collect();
                if !failed.is_empty() {
                    println!();
                    println!("  \x1b[1;31m⚠ Failed:\x1b[0m");
                    for f in &failed {
                        println!("    {}", f);
                    }
                }
            }
            let _ = app.reload_stack();
            // Save sync state for divergence detection
            if let Ok(r) = repo_loader::open_resolved() {
                app.forge.save_sync_state(&r, &app.stack.patches);
            }
            app.notify(format!("Synced {} PRs.", updates.len()));
        }
        Err(e) => {
            clear_screen();
            println!();
            println!("  \x1b[31m✗ Sync failed: {}\x1b[0m", e);
            app.notify(format!("Sync failed: {}", e));
        }
    }

    // Check for stale branches
    prompt_cleanup_stale_branches(app)?;

    println!();
    println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
    wait_for_enter()?;
    Ok(())
}

/// Pull remote changes into local stack, then sync.
fn handle_pull_remote(app: &mut App) -> Result<()> {
    clear_screen();
    println!();
    println!("  \x1b[1;36m▸ Pulling remote changes...\x1b[0m");
    println!();

    let repo = repo_loader::open_resolved()?;
    let working_branch = repo.get_current_branch()?;
    println!("    \x1b[33mFetching origin...\x1b[0m");
    let _ = repo.fetch_origin();

    // Find diverged branches
    let patches = app.stack.patches.clone();
    let diverged = app.forge.check_diverged(&repo, &patches);

    if diverged.is_empty() {
        println!();
        println!("  \x1b[32m✓ All PRs are in sync with remote.\x1b[0m");
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        app.notify("No remote changes to pull.");
        return Ok(());
    }

    println!("  \x1b[33m▸ Found {} diverged PRs:\x1b[0m", diverged.len());
    for (_branch, desc) in &diverged {
        println!("    \x1b[33m{}\x1b[0m", desc);
    }
    println!();

    // Build set of diverged subjects and collect remote refs BEFORE rebasing
    let diverged_branches: std::collections::HashSet<String> =
        diverged.iter().map(|(b, _)| b.clone()).collect();

    let mut subject_to_remote: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for patch in patches.iter() {
        if patch.status != crate::core::stack::PatchStatus::Submitted {
            continue;
        }
        let branch = repo.make_pgit_branch_name(&patch.subject);
        let is_diverged = diverged_branches.contains(&branch)
            || patch
                .pr_number
                .map(|n| diverged_branches.contains(&format!("D{}", n)))
                .unwrap_or(false);
        if !is_diverged {
            continue;
        }

        if let Some(remote_ref) = app.forge.get_remote_ref(&repo, patch) {
            subject_to_remote.insert(patch.subject.clone(), remote_ref);
        }
    }

    // Restore working branch if get_remote_ref changed it (e.g. Phabricator arc patch)
    let _ = repo.git_pub(&["checkout", "--quiet", &working_branch]);

    if subject_to_remote.is_empty() {
        println!("  \x1b[31m⚠ Could not get remote refs for any diverged PRs.\x1b[0m");
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        return Ok(());
    }

    // Single rebase pass: mark all commits as "edit"
    let base = repo.base()?;
    println!("    \x1b[33mRebasing to merge remote changes...\x1b[0m");

    let rebase_output = Command::new("git")
        .current_dir(&repo.workdir)
        .args(["rebase", "-i", &base])
        .env(
            "GIT_SEQUENCE_EDITOR",
            format!("{} 's/^pick /edit /'", sed_inplace_shell_prefix()),
        )
        .output();

    if !repo.is_rebase_in_progress() {
        let reason = match &rebase_output {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !stderr.is_empty() {
                    stderr
                } else if !stdout.is_empty() {
                    stdout
                } else {
                    "No commits to rebase.".to_string()
                }
            }
            Err(e) => format!("git rebase failed: {}", e),
        };
        println!("  \x1b[31m⚠ Rebase could not start: {}\x1b[0m", reason);
        let _ = repo.git_pub(&["checkout", "--quiet", &working_branch]);
        println!();
        println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
        wait_for_enter()?;
        return Ok(());
    }

    // Process each commit in the rebase
    let mut merged_count = 0;
    loop {
        if !repo.is_rebase_in_progress() {
            break;
        }

        // Get current commit's subject to identify it
        let subject = repo
            .git_pub(&["log", "-1", "--format=%s"])
            .unwrap_or_default()
            .trim()
            .to_string();

        // Check if this commit has a remote ref to merge
        if let Some(remote_ref) = subject_to_remote.get(&subject) {
            println!("    \x1b[36mMerging remote changes for: {}\x1b[0m", subject);

            // Merge remote changes into our commit (preserves both local and remote edits)
            let merge_result = Command::new("git")
                .current_dir(&repo.workdir)
                .args(["merge", "--squash", "--no-commit", remote_ref])
                .output();

            let has_conflict = match &merge_result {
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    !out.status.success() || stderr.contains("CONFLICT")
                }
                Err(_) => true,
            };

            if has_conflict {
                println!();
                println!("    \x1b[1;33m⚠ Merge conflict while pulling remote changes.\x1b[0m");
                println!("    Resolve conflicts, then stage: \x1b[1;33mgit add <files>\x1b[0m");
                println!();
                println!(
                    "    Press \x1b[1;32mc\x1b[0m to continue  or  \x1b[1;31ma\x1b[0m to abort"
                );

                loop {
                    let choice = read_single_key()?;
                    match choice {
                        'c' => {
                            let _ = Command::new("git")
                                .current_dir(&repo.workdir)
                                .args(["add", "-A"])
                                .output();
                            let _ = Command::new("git")
                                .current_dir(&repo.workdir)
                                .args(["commit", "--amend", "--no-edit"])
                                .output();
                            break;
                        }
                        'a' => {
                            let _ = repo.rebase_abort();
                            for ref_name in subject_to_remote.values() {
                                if ref_name.starts_with("pgit-temp-patch-") {
                                    let _ = repo.git_pub(&["branch", "-D", ref_name]);
                                }
                            }
                            println!("    \x1b[31mAborted.\x1b[0m");
                            println!();
                            println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
                            wait_for_enter()?;
                            let _ = app.reload_stack();
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            } else {
                // Clean merge — stage and amend
                let _ = Command::new("git")
                    .current_dir(&repo.workdir)
                    .args(["add", "-A"])
                    .output();
                let _ = Command::new("git")
                    .current_dir(&repo.workdir)
                    .args(["commit", "--amend", "--no-edit"])
                    .output();
            }

            merged_count += 1;
        }

        // Continue to next commit
        match repo.rebase_continue() {
            Ok(true) => break, // rebase completed
            Ok(false) => {
                if !repo.is_rebase_in_progress() {
                    break;
                }
                // Could be a conflict during replay or next edit pause
                // If conflict, let user resolve
                let conflicts = repo.conflicted_files().unwrap_or_default();
                if !conflicts.is_empty() {
                    println!("    \x1b[1;33m⚠ Conflict during rebase.\x1b[0m");
                    println!("    Resolve conflicts, stage, then press \x1b[1;32mc\x1b[0m or \x1b[1;31ma\x1b[0m");
                    loop {
                        let choice = read_single_key()?;
                        match choice {
                            'c' => {
                                let _ = Command::new("git")
                                    .current_dir(&repo.workdir)
                                    .args(["add", "-A"])
                                    .output();
                                match repo.rebase_continue() {
                                    Ok(true) => break,
                                    Ok(false) => {
                                        if !repo.is_rebase_in_progress() {
                                            break;
                                        }
                                        continue;
                                    }
                                    Err(_) => {
                                        let _ = repo.rebase_abort();
                                        break;
                                    }
                                }
                            }
                            'a' => {
                                let _ = repo.rebase_abort();
                                println!("    \x1b[31mAborted.\x1b[0m");
                                println!();
                                println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
                                wait_for_enter()?;
                                let _ = app.reload_stack();
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }
                // Otherwise it's paused at the next "edit" — continue loop
            }
            Err(_) => {
                let _ = repo.rebase_abort();
                break;
            }
        }
    }

    // Clean up Phabricator temp branches
    for ref_name in subject_to_remote.values() {
        if ref_name.starts_with("pgit-temp-patch-") {
            let _ = repo.git_pub(&["branch", "-D", ref_name]);
        }
    }

    // Restore working branch in case rebase left us elsewhere
    let _ = repo.git_pub(&["checkout", "--quiet", &working_branch]);

    let _ = app.reload_stack();

    clear_screen();
    println!();
    println!(
        "  \x1b[32m✓ Pulled remote changes for {} PRs.\x1b[0m",
        merged_count
    );
    println!();
    println!("  You can now review, make more changes, and press \x1b[1;32ms\x1b[0m to sync.");

    // Save sync state so divergence check won't re-flag these changes
    if let Ok(r) = repo_loader::open_resolved() {
        app.forge.save_sync_state(&r, &app.stack.patches);
    }

    println!();
    println!("  Press \x1b[1;32mEnter\x1b[0m to return.");
    wait_for_enter()?;
    app.notify(format!("Pulled remote changes for {} PRs.", merged_count));
    Ok(())
}

/// Check for stale pgit branches (merged/closed PRs) and ask the user
/// if they want to delete them (local + remote).
fn prompt_cleanup_stale_branches(app: &App) -> Result<()> {
    let repo = repo_loader::open_resolved()?;
    let (open_prs, gh_avail) = app.forge.list_open(&repo);
    let mut stale = repo.find_stale_branches_with(&open_prs, gh_avail);

    // Also check forge-specific landed branches (e.g. Phabricator trailer match)
    let all_branches = repo.list_pgit_branches();
    let landed = app.forge.find_landed_branches(&repo, &all_branches);
    for b in landed {
        if !stale.contains(&b) {
            stale.push(b);
        }
    }

    if stale.is_empty() {
        return Ok(());
    }

    println!();
    println!(
        "  \x1b[33m▸ Found {} stale branches (PR merged or closed):\x1b[0m",
        stale.len()
    );
    for b in &stale {
        println!("    {}", b);
    }
    println!();
    print!("  Delete these branches (local + remote)? \x1b[1;32my\x1b[0m/\x1b[1;31mn\x1b[0m  ");
    io::stdout().flush()?;

    let choice = read_single_key()?;
    println!();

    if choice == 'y' {
        println!("  Deleting...");
        for b in &stale {
            println!("    \x1b[33m{}\x1b[0m", b);
        }
        repo.delete_branches(&stale);
        println!("  \x1b[32m✓ Deleted {} stale branches.\x1b[0m", stale.len());
    } else {
        println!("  Skipped.");
    }
    println!();

    Ok(())
}

/// Handle rebase conflicts — single-keypress resolution loop.
fn handle_rebase_conflict(app: &mut App) -> Result<()> {
    loop {
        let repo = repo_loader::open_resolved()?;
        let conflicts = repo.conflicted_files().unwrap_or_default();

        // No more conflicts — continue automatically
        if conflicts.is_empty() {
            println!("  No remaining conflicts. Continuing rebase...");
            match app.continue_rebase() {
                Ok(true) => return Ok(()),
                Ok(false) => continue, // new conflicts from next commit
                Err(e) => {
                    app.notify(format!("Continue failed: {}", e));
                    return Ok(());
                }
            }
        }

        let mut lines: Vec<String> = vec![];
        lines.push("Conflicting files:".into());
        for f in &conflicts {
            lines.push(format!("  \x1b[1;33m{}\x1b[0m", f));
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
