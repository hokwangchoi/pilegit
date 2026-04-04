use color_eyre::Result;
use crossterm::event::{self, Event, KeyEvent};
use std::time::Duration;

use super::input;
use super::ui;
use super::Tui;
use crate::core::history::History;
use crate::core::stack::Stack;

const HELP_MSG: &str =
    "j/k: move | V: select | s: squash | K/J: reorder | d: diff | i: insert | x: drop | u: undo | q: quit";

/// Interaction mode for the TUI.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Normal navigation
    Normal,
    /// Visual selection — j/k to extend selection
    Select,
    /// Viewing diff of a commit
    DiffView,
    /// Viewing undo history
    HistoryView,
    /// Confirm dialog (e.g. before squash)
    Confirm {
        prompt: String,
        action: PendingAction,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingAction {
    Squash,
    Drop,
}

pub struct App {
    pub stack: Stack,
    pub history: History,
    pub mode: Mode,
    /// Cursor position in the patch list (0 = bottom of stack)
    pub cursor: usize,
    /// Selection anchor for visual mode (inclusive range anchor..cursor)
    pub select_anchor: Option<usize>,
    /// Currently expanded commit (showing details)
    pub expanded: Option<usize>,
    /// Scroll offset for the list view
    pub scroll_offset: usize,
    /// Diff content when in DiffView mode
    pub diff_content: Vec<String>,
    pub diff_scroll: usize,
    /// Status bar message
    pub status_msg: String,
    /// Should quit
    pub should_quit: bool,
}

impl App {
    pub fn new(stack: Stack) -> Self {
        let cursor = if stack.is_empty() {
            0
        } else {
            stack.len() - 1
        };
        let mut history = History::new(100);
        // Record the initial state so we can undo back to it
        history.push("initial", &stack);
        Self {
            stack,
            history,
            mode: Mode::Normal,
            cursor,
            select_anchor: None,
            expanded: None,
            scroll_offset: 0,
            diff_content: Vec::new(),
            diff_scroll: 0,
            status_msg: HELP_MSG.to_string(),
            should_quit: false,
        }
    }

    /// Main event loop.
    pub fn run(&mut self, terminal: &mut Tui) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::render(frame, self))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            Mode::Normal => input::handle_normal(self, key),
            Mode::Select => input::handle_select(self, key),
            Mode::DiffView => input::handle_diff_view(self, key),
            Mode::HistoryView => input::handle_history_view(self, key),
            Mode::Confirm { .. } => input::handle_confirm(self, key),
        }
    }

    /// Get the selected range (inclusive), always ordered low..=high.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.select_anchor.map(|anchor| {
            let lo = anchor.min(self.cursor);
            let hi = anchor.max(self.cursor);
            (lo, hi)
        })
    }

    /// Record current state after an operation for undo/redo.
    fn record(&mut self, description: &str) {
        self.history.push(description, &self.stack);
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.history.undo() {
            self.stack = prev.clone();
            self.clamp_cursor();
            self.status_msg = format!(
                "Undone. ({}/{})",
                self.history.position(),
                self.history.total()
            );
        } else {
            self.status_msg = "Nothing to undo.".into();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.history.redo() {
            self.stack = next.clone();
            self.clamp_cursor();
            self.status_msg = format!(
                "Redone. ({}/{})",
                self.history.position(),
                self.history.total()
            );
        } else {
            self.status_msg = "Nothing to redo.".into();
        }
    }

    pub fn clamp_cursor(&mut self) {
        if self.stack.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.stack.len() {
            self.cursor = self.stack.len() - 1;
        }
    }

    pub fn move_cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_cursor_down(&mut self) {
        if !self.stack.is_empty() && self.cursor < self.stack.len() - 1 {
            self.cursor += 1;
        }
    }

    pub fn move_patch_up(&mut self) {
        if self.cursor > 0 && !self.stack.is_empty() {
            let _ = self.stack.reorder(self.cursor, self.cursor - 1);
            self.cursor -= 1;
            self.record("reorder patch up");
            self.status_msg = "Patch moved up.".into();
        }
    }

    pub fn move_patch_down(&mut self) {
        if !self.stack.is_empty() && self.cursor < self.stack.len() - 1 {
            let _ = self.stack.reorder(self.cursor, self.cursor + 1);
            self.cursor += 1;
            self.record("reorder patch down");
            self.status_msg = "Patch moved down.".into();
        }
    }

    pub fn squash_selected(&mut self) {
        if let Some((lo, hi)) = self.selection_range() {
            let indices: Vec<usize> = (lo..=hi).collect();
            let count = indices.len();
            match self.stack.squash(&indices) {
                Ok(()) => {
                    self.record("squash commits");
                    self.select_anchor = None;
                    self.mode = Mode::Normal;
                    self.cursor = lo;
                    self.clamp_cursor();
                    self.status_msg = format!("Squashed {} commits.", count);
                }
                Err(e) => {
                    self.status_msg = format!("Squash failed: {}", e);
                }
            }
        } else {
            self.status_msg = "No selection. Use V or Shift+arrows to select.".into();
        }
    }

    pub fn drop_at_cursor(&mut self) {
        if self.stack.is_empty() {
            return;
        }
        match self.stack.drop_patch(self.cursor) {
            Ok(dropped) => {
                self.record("drop commit");
                self.clamp_cursor();
                self.status_msg = format!("Dropped: {}", dropped.subject);
            }
            Err(e) => {
                self.status_msg = format!("Drop failed: {}", e);
            }
        }
    }

    pub fn insert_commit_at_cursor(&mut self) {
        self.status_msg =
            "Insert: suspend TUI, make changes, commit, then `exit` to return.".into();
        // TODO: terminal suspend → spawn $SHELL → detect new commits → resume TUI
    }

    pub fn reset_status(&mut self) {
        self.status_msg = HELP_MSG.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stack::PatchEntry;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn make_app(n: usize) -> App {
        let patches: Vec<PatchEntry> = (0..n)
            .map(|i| PatchEntry::new(&format!("h{:07}", i), &format!("commit {}", i)))
            .collect();
        App::new(Stack::new("main".into(), patches))
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    // --- cursor ---

    #[test]
    fn test_initial_cursor_at_top_of_stack() {
        let app = make_app(5);
        assert_eq!(app.cursor, 4); // newest commit
    }

    #[test]
    fn test_initial_cursor_empty_stack() {
        let app = make_app(0);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_move_cursor_down_stops_at_bottom() {
        let mut app = make_app(3);
        app.cursor = 2;
        app.move_cursor_down();
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn test_move_cursor_up_stops_at_top() {
        let mut app = make_app(3);
        app.cursor = 0;
        app.move_cursor_up();
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_cursor_navigation_via_keys() {
        let mut app = make_app(5);
        assert_eq!(app.cursor, 4);
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.cursor, 3);
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.cursor, 2);
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.cursor, 3);
    }

    #[test]
    fn test_jump_to_top_and_bottom() {
        let mut app = make_app(5);
        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.cursor, 0);
        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.cursor, 4);
    }

    // --- select mode ---

    #[test]
    fn test_enter_visual_select() {
        let mut app = make_app(5);
        app.cursor = 2;
        app.handle_key(key(KeyCode::Char('V')));
        assert_eq!(app.mode, Mode::Select);
        assert_eq!(app.select_anchor, Some(2));
    }

    #[test]
    fn test_select_and_cancel() {
        let mut app = make_app(5);
        app.cursor = 2;
        app.handle_key(key(KeyCode::Char('V')));
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.select_anchor, None);
    }

    #[test]
    fn test_selection_range_ordered() {
        let mut app = make_app(5);
        app.cursor = 3;
        app.select_anchor = Some(1);
        assert_eq!(app.selection_range(), Some((1, 3)));

        // Reversed anchor
        app.cursor = 1;
        app.select_anchor = Some(3);
        assert_eq!(app.selection_range(), Some((1, 3)));
    }

    // --- squash ---

    #[test]
    fn test_squash_via_select() {
        let mut app = make_app(5);
        app.cursor = 1;
        app.select_anchor = Some(1);
        app.mode = Mode::Select;

        // Extend selection down
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.cursor, 2);
        assert_eq!(app.selection_range(), Some((1, 2)));

        // Trigger squash confirm
        app.handle_key(key(KeyCode::Char('s')));
        assert!(matches!(app.mode, Mode::Confirm { .. }));

        // Confirm
        app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.stack.len(), 4); // 5 - 1 squashed
    }

    // --- reorder ---

    #[test]
    fn test_reorder_patch_up() {
        let mut app = make_app(4);
        app.cursor = 2;
        app.handle_key(key(KeyCode::Char('K'))); // move up
        assert_eq!(app.cursor, 1);
        assert_eq!(app.stack.patches[1].subject, "commit 2");
        assert_eq!(app.stack.patches[2].subject, "commit 1");
    }

    #[test]
    fn test_reorder_patch_down() {
        let mut app = make_app(4);
        app.cursor = 1;
        app.handle_key(key(KeyCode::Char('J'))); // move down
        assert_eq!(app.cursor, 2);
        assert_eq!(app.stack.patches[1].subject, "commit 2");
        assert_eq!(app.stack.patches[2].subject, "commit 1");
    }

    // --- drop ---

    #[test]
    fn test_drop_with_confirm() {
        let mut app = make_app(3);
        app.cursor = 1;
        app.handle_key(key(KeyCode::Char('x'))); // trigger drop
        assert!(matches!(app.mode, Mode::Confirm { .. }));

        app.handle_key(key(KeyCode::Char('y'))); // confirm
        assert_eq!(app.stack.len(), 2);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_drop_cancel() {
        let mut app = make_app(3);
        app.cursor = 1;
        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('n'))); // cancel
        assert_eq!(app.stack.len(), 3); // unchanged
    }

    // --- undo/redo ---

    #[test]
    fn test_undo_redo() {
        let mut app = make_app(4);
        let original_len = app.stack.len();
        app.cursor = 1;

        // Drop a commit
        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(app.stack.len(), 3);

        // Undo
        app.handle_key(key(KeyCode::Char('u')));
        assert_eq!(app.stack.len(), original_len);

        // Redo
        app.handle_key(key_ctrl(KeyCode::Char('r')));
        assert_eq!(app.stack.len(), 3);
    }

    #[test]
    fn test_undo_empty_history() {
        let mut app = make_app(3);
        app.handle_key(key(KeyCode::Char('u')));
        assert!(app.status_msg.contains("Nothing to undo"));
    }

    // --- clamp ---

    #[test]
    fn test_clamp_after_drop_last() {
        let mut app = make_app(3);
        app.cursor = 2;
        app.drop_at_cursor();
        assert!(app.cursor <= app.stack.len().saturating_sub(1));
    }

    #[test]
    fn test_clamp_on_empty() {
        let mut app = make_app(1);
        app.drop_at_cursor();
        assert_eq!(app.cursor, 0);
        assert!(app.stack.is_empty());
    }

    // --- quit ---

    #[test]
    fn test_quit() {
        let mut app = make_app(3);
        assert!(!app.should_quit);
        app.handle_key(key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    // --- expand ---

    #[test]
    fn test_expand_collapse() {
        let mut app = make_app(3);
        app.cursor = 1;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.expanded, Some(1));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.expanded, None);
    }

    // --- shift+arrow enters select ---

    #[test]
    fn test_shift_down_enters_select() {
        let mut app = make_app(5);
        app.cursor = 2;
        app.handle_key(key_shift(KeyCode::Down));
        assert_eq!(app.mode, Mode::Select);
        assert_eq!(app.select_anchor, Some(2));
        assert_eq!(app.cursor, 3);
    }

    // --- history view ---

    #[test]
    fn test_history_view_enter_and_exit() {
        let mut app = make_app(3);
        app.handle_key(key(KeyCode::Char('h')));
        assert_eq!(app.mode, Mode::HistoryView);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
    }
}
