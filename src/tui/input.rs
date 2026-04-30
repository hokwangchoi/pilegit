use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Mode, PendingAction};

/// Handle keys in Normal mode.
///
/// Modifier priority: Shift+Arrow → select, Alt → reorder, Ctrl+r → redo.
/// Capital letters (Shift+letter) are NOT intercepted by the Shift block —
/// they fall through to the plain key match so R, S, G, V all work.
pub fn handle_normal(app: &mut App, key: KeyEvent) {
    // Shift + Arrow keys only: enter select mode
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Up => {
                app.clear_notification();
                app.select_anchor = Some(app.cursor);
                app.mode = Mode::Select;
                app.move_cursor_up();
                return;
            }
            KeyCode::Down => {
                app.clear_notification();
                app.select_anchor = Some(app.cursor);
                app.mode = Mode::Select;
                app.move_cursor_down();
                return;
            }
            // Capital letters and other Shift combos fall through
            _ => {}
        }
    }

    // Alt combos: not used currently
    if key.modifiers.contains(KeyModifiers::ALT) {
        return;
    }

    // Ctrl combos: reorder (Ctrl+arrow/jk) and redo (Ctrl+r)
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.move_patch_up(),
            KeyCode::Down | KeyCode::Char('j') => app.move_patch_down(),
            KeyCode::Char('r') => app.redo(),
            _ => {}
        }
        return;
    }

    // Plain keys (and Shift+letter which gives uppercase Char)
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,

        // Navigation
        KeyCode::Char('k') | KeyCode::Up => app.move_cursor_up(),
        KeyCode::Char('j') | KeyCode::Down => app.move_cursor_down(),
        KeyCode::Char('g') => {
            app.clear_notification();
            if !app.stack.is_empty() { app.cursor = app.stack.len() - 1; }
        }
        KeyCode::Char('G') => { app.clear_notification(); app.cursor = 0; }

        // Visual select
        KeyCode::Char('V') => {
            app.clear_notification();
            app.select_anchor = Some(app.cursor);
            app.mode = Mode::Select;
        }

        // Expand/collapse
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.clear_notification();
            if app.expanded == Some(app.cursor) { app.expanded = None; }
            else { app.expanded = Some(app.cursor); }
        }

        // Diff view
        KeyCode::Char('d') => {
            app.clear_notification();
            if !app.stack.is_empty() {
                let hash = app.stack.patches[app.cursor].hash.clone();
                match crate::git::repo_loader::open_resolved().and_then(|r| r.diff_full(&hash)) {
                    Ok(diff) => {
                        app.diff_content = diff.lines().map(|l| l.to_string()).collect();
                        app.diff_scroll = 0;
                        app.mode = Mode::DiffView;
                    }
                    Err(e) => app.notify(format!("diff error: {}", e)),
                }
            }
        }

        // Edit/amend commit
        KeyCode::Char('e') => app.edit_commit_at_cursor(),

        // Insert — show choice prompt
        KeyCode::Char('i') => app.show_insert_choice(),

        // Remove commit (with confirm)
        KeyCode::Char('x') => {
            if !app.stack.is_empty() {
                let subject = app.stack.patches[app.cursor].subject.clone();
                app.mode = Mode::Confirm {
                    prompt: format!("Remove '{}'? (y/n)", subject),
                    action: PendingAction::Drop,
                };
            }
        }

        // Rebase onto base branch
        KeyCode::Char('r') => app.start_rebase(),

        // Publish/submit via custom command or GitHub
        KeyCode::Char('p') => app.submit_at_cursor(),

        // Pull remote changes into local commits
        KeyCode::Char('P') => app.pull_remote(),

        // Sync all submitted PRs (force-push + update bases)
        KeyCode::Char('s') => app.sync_all_prs(),

        // Refresh stack display (re-reads commits from git)
        KeyCode::Char('R') => {
            match app.reload_stack() {
                Ok(()) => app.notify("Stack refreshed."),
                Err(e) => app.notify(format!("Refresh failed: {}", e)),
            }
        }

        // Undo
        KeyCode::Char('u') => app.undo(),

        // History view
        KeyCode::Char('h') => { app.clear_notification(); app.mode = Mode::HistoryView; }

        // Help
        KeyCode::Char('?') => app.show_help(),

        _ => {}
    }
}

/// Handle keys in Select (visual) mode.
pub fn handle_select(app: &mut App, key: KeyEvent) {
    // Shift+arrows also extend selection
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Up => { app.move_cursor_up(); return; }
            KeyCode::Down => { app.move_cursor_down(); return; }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('k') | KeyCode::Up => app.move_cursor_up(),
        KeyCode::Char('j') | KeyCode::Down => app.move_cursor_down(),

        KeyCode::Char('s') => {
            if let Some((lo, hi)) = app.selection_range() {
                let count = hi - lo + 1;
                app.mode = Mode::Confirm {
                    prompt: format!("Squash {} commits? (y/n)", count),
                    action: PendingAction::Squash,
                };
            }
        }

        KeyCode::Esc | KeyCode::Char('q') => {
            app.select_anchor = None;
            app.mode = Mode::Normal;
        }

        _ => {}
    }
}

/// Handle keys in DiffView mode.
pub fn handle_diff_view(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                app.diff_scroll = app.diff_scroll.saturating_add(20)
                    .min(app.diff_content.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => { app.diff_scroll = app.diff_scroll.saturating_sub(20); }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.diff_content.clear();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.diff_scroll < app.diff_content.len().saturating_sub(1) {
                app.diff_scroll += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => { app.diff_scroll = app.diff_scroll.saturating_sub(1); }
        _ => {}
    }
}

pub fn handle_history_view(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
        app.mode = Mode::Normal;
    }
}

pub fn handle_help(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?')) {
        app.mode = Mode::Normal;
    }
}

/// Handle the insert location choice prompt.
pub fn handle_insert_choice(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('a') => app.insert_after_cursor(),
        KeyCode::Char('t') => app.insert_at_head(),
        KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Normal; }
        _ => {}
    }
}

/// Handle keys in Confirm dialog mode.
pub fn handle_confirm(app: &mut App, key: KeyEvent) {
    let (action, confirmed) = match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            if let Mode::Confirm { ref action, .. } = app.mode {
                (Some(action.clone()), true)
            } else { (None, false) }
        }
        KeyCode::Char('n') | KeyCode::Esc => (None, false),
        _ => return,
    };

    app.mode = Mode::Normal;

    if confirmed {
        if let Some(action) = action {
            match action {
                PendingAction::Squash => app.squash_selected(),
                PendingAction::Drop => app.drop_at_cursor(),
                PendingAction::Rebase => {
                    if let Err(e) = app.execute_rebase() {
                        app.notify(format!("Rebase error: {}", e));
                    }
                }
            }
        }
    } else {
        app.notify("Cancelled.");
    }
}
