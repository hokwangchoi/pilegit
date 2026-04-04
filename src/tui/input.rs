use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Mode, PendingAction};

/// Handle keys in Normal mode.
pub fn handle_normal(app: &mut App, key: KeyEvent) {
    // Modifier combos first
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Up => {
                app.select_anchor = Some(app.cursor);
                app.mode = Mode::Select;
                app.move_cursor_up();
                app.status_msg = "SELECT: ↑↓/jk extend | s: squash | Esc: cancel".into();
            }
            KeyCode::Down => {
                app.select_anchor = Some(app.cursor);
                app.mode = Mode::Select;
                app.move_cursor_down();
                app.status_msg = "SELECT: ↑↓/jk extend | s: squash | Esc: cancel".into();
            }
            _ => {}
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Up => app.move_patch_up(),
            KeyCode::Down => app.move_patch_down(),
            _ => {}
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('r') = key.code {
            app.redo();
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,

        // Navigation (visual: up=newer/higher index, down=older/lower index)
        KeyCode::Char('k') | KeyCode::Up => app.move_cursor_up(),
        KeyCode::Char('j') | KeyCode::Down => app.move_cursor_down(),

        // g = top of screen (newest), G = bottom (oldest)
        KeyCode::Char('g') => {
            if !app.stack.is_empty() {
                app.cursor = app.stack.len() - 1;
            }
        }
        KeyCode::Char('G') => {
            app.cursor = 0;
        }

        // Visual select
        KeyCode::Char('V') => {
            app.select_anchor = Some(app.cursor);
            app.mode = Mode::Select;
            app.status_msg = "SELECT: ↑↓/jk extend | s: squash | Esc: cancel".into();
        }

        // Reorder: K = move patch up (visual), J = move patch down (visual)
        KeyCode::Char('K') => app.move_patch_up(),
        KeyCode::Char('J') => app.move_patch_down(),

        // Expand/collapse commit detail
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.expanded == Some(app.cursor) {
                app.expanded = None;
            } else {
                app.expanded = Some(app.cursor);
            }
        }

        // View full diff
        KeyCode::Char('d') => {
            if !app.stack.is_empty() {
                let hash = app.stack.patches[app.cursor].hash.clone();
                match crate::git::ops::Repo::open().and_then(|r| r.diff_full(&hash)) {
                    Ok(diff) => {
                        app.diff_content = diff.lines().map(|l| l.to_string()).collect();
                        app.diff_scroll = 0;
                        app.mode = Mode::DiffView;
                        app.status_msg = "DIFF: ↑↓/jk scroll | Ctrl+d/u half-page | q: back".into();
                    }
                    Err(e) => app.status_msg = format!("diff error: {}", e),
                }
            }
        }

        // Insert new commit
        KeyCode::Char('i') => app.insert_commit(),

        // Drop commit
        KeyCode::Char('x') => {
            if !app.stack.is_empty() {
                let subject = app.stack.patches[app.cursor].subject.clone();
                app.mode = Mode::Confirm {
                    prompt: format!("Drop '{}'? (y/n)", subject),
                    action: PendingAction::Drop,
                };
            }
        }

        // Rebase onto base branch
        KeyCode::Char('R') => app.start_rebase(),

        // Submit current commit via custom command
        KeyCode::Char('S') => app.submit_at_cursor(),

        // Undo
        KeyCode::Char('u') => app.undo(),

        // History view
        KeyCode::Char('h') => {
            app.mode = Mode::HistoryView;
            app.status_msg = "HISTORY: q/Esc to go back".into();
        }

        _ => {}
    }
}

/// Handle keys in Select (visual) mode.
pub fn handle_select(app: &mut App, key: KeyEvent) {
    match key.code {
        // Extend selection (same visual direction as normal mode)
        KeyCode::Char('k') | KeyCode::Up => app.move_cursor_up(),
        KeyCode::Char('j') | KeyCode::Down => app.move_cursor_down(),

        // Squash selected
        KeyCode::Char('s') => {
            if let Some((lo, hi)) = app.selection_range() {
                let count = hi - lo + 1;
                app.mode = Mode::Confirm {
                    prompt: format!("Squash {} commits? (y/n)", count),
                    action: PendingAction::Squash,
                };
            }
        }

        // Cancel selection
        KeyCode::Esc | KeyCode::Char('q') => {
            app.select_anchor = None;
            app.mode = Mode::Normal;
            app.reset_status();
        }

        _ => {}
    }
}

/// Handle keys in DiffView mode.
pub fn handle_diff_view(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('d') => {
                app.diff_scroll = app
                    .diff_scroll
                    .saturating_add(20)
                    .min(app.diff_content.len().saturating_sub(1));
            }
            KeyCode::Char('u') => {
                app.diff_scroll = app.diff_scroll.saturating_sub(20);
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.diff_content.clear();
            app.reset_status();
        }
        // In diff view, j/k scroll content (not reversed — j=down in text)
        KeyCode::Char('j') | KeyCode::Down => {
            if app.diff_scroll < app.diff_content.len().saturating_sub(1) {
                app.diff_scroll += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.diff_scroll = app.diff_scroll.saturating_sub(1);
        }
        _ => {}
    }
}

/// Handle keys in HistoryView mode.
pub fn handle_history_view(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.reset_status();
        }
        _ => {}
    }
}

/// Handle keys in Confirm dialog mode.
pub fn handle_confirm(app: &mut App, key: KeyEvent) {
    let (action, confirmed) = match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            if let Mode::Confirm { ref action, .. } = app.mode {
                (Some(action.clone()), true)
            } else {
                (None, false)
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => (None, false),
        _ => return,
    };

    // Always leave Confirm mode
    app.mode = Mode::Normal;

    if confirmed {
        if let Some(action) = action {
            match action {
                PendingAction::Squash => app.squash_selected(),
                PendingAction::Drop => app.drop_at_cursor(),
                PendingAction::Rebase => {
                    match app.execute_rebase() {
                        Ok(true) => {} // clean rebase, message already set
                        Ok(false) => {} // conflicts, suspend will happen
                        Err(e) => app.status_msg = format!("Rebase error: {}", e),
                    }
                }
            }
        }
    } else {
        app.status_msg = "Cancelled.".into();
    }
}
