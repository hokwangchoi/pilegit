use color_eyre::Result;
use crossterm::event::{self, Event, KeyEvent};
use std::time::Duration;

use super::input;
use super::ui;
use super::Tui;
use crate::core::history::History;
use crate::core::stack::Stack;
use crate::forge::Forge;

/// Why the TUI is being suspended.
#[derive(Debug, Clone, PartialEq)]
pub enum SuspendReason {
    InsertAtHead,
    InsertAfterCursor { hash: String },
    EditCommit { hash: String },
    /// Squash commits: edit the message first, then perform git squash.
    SquashCommits {
        /// Short hashes of commits to squash (first = target, rest = folded in)
        hashes: Vec<String>,
        default_body: String,
        /// Forge-specific trailers to preserve from squashed commits
        trailers: Vec<String>,
    },
    /// Submit a commit as a PR — opens editor for PR description first
    SubmitCommit {
        hash: String,
        subject: String,
        cursor_index: usize,
    },
    /// Update an existing PR — force-push and update base
    UpdatePR {
        hash: String,
        subject: String,
        cursor_index: usize,
    },
    RebaseConflict,
    /// Sync all submitted PRs — suspend TUI to show progress
    SyncPRs,
    /// Pull remote changes into local commits — suspend TUI
    PullRemote,
    /// Rebase onto base — suspend TUI to show progress
    Rebase,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Select,
    DiffView,
    HistoryView,
    Help,
    /// Prompt: insert (a)fter cursor or at (t)op?
    InsertChoice,
    Confirm { prompt: String, action: PendingAction },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingAction {
    Squash,
    Drop,
    Rebase,
}

pub struct App {
    pub stack: Stack,
    pub history: History,
    pub mode: Mode,
    pub cursor: usize,
    pub select_anchor: Option<usize>,
    pub expanded: Option<usize>,
    pub diff_content: Vec<String>,
    pub diff_scroll: usize,
    pub notification: Option<String>,
    pub should_quit: bool,
    pub wants_suspend: Option<SuspendReason>,
    /// The code review platform integration.
    pub forge: Box<dyn Forge>,
}

impl App {
    pub fn new(stack: Stack, forge: Box<dyn Forge>) -> Self {
        let cursor = if stack.is_empty() { 0 } else { stack.len() - 1 };
        let mut history = History::new(500);
        // Record initial state with current HEAD
        let head = Self::current_head().unwrap_or_default();
        history.push("initial state", &stack, &head);
        Self {
            stack, history,
            mode: Mode::Normal,
            cursor,
            select_anchor: None,
            expanded: None,
            diff_content: Vec::new(),
            diff_scroll: 0,
            notification: None,
            should_quit: false,
            wants_suspend: None,
            forge,
        }
    }

    pub fn run(&mut self, terminal: &mut Tui) -> Result<()> {
        while !self.should_quit {
            if self.wants_suspend.is_some() { return Ok(()); }
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
            Mode::Help => input::handle_help(self, key),
            Mode::InsertChoice => input::handle_insert_choice(self, key),
            Mode::Confirm { .. } => input::handle_confirm(self, key),
        }
    }

    /// Shortcut hints for the current mode (always shown at bottom).
    pub fn shortcuts(&self) -> &str {
        match &self.mode {
            Mode::Normal => "↑k/↓j:move  V/Shift+↑↓:select  Ctrl+↑↓:reorder  e:edit  i:insert  x:remove  d:diff  r:rebase  p:submit/update PR  s:sync  P:pull remote  R:refresh  u:undo  Ctrl+r:redo  ?:help  q:quit",
            Mode::Select => "Shift+↑↓ or j/k:extend selection  s:squash  Esc:cancel",
            Mode::DiffView => "↑k/↓j:scroll  Ctrl+↑↓:half-page  q/Esc:back",
            Mode::HistoryView => "q/Esc:back",
            Mode::Help => "q/Esc:close",
            Mode::InsertChoice => "a:insert after cursor  t:insert at top  Esc:cancel",
            Mode::Confirm { .. } => "y:confirm  n/Esc:cancel",
        }
    }

    pub fn help_text(&self) -> &'static str {
        "\
 NAVIGATION
   ↑ / k           Move cursor up (toward newer)
   ↓ / j           Move cursor down (toward older)
   g               Jump to top (newest commit)
   G               Jump to bottom (oldest commit)
   Enter / Space   Expand or collapse commit details

 SELECTION & SQUASH
   V               Start visual selection at cursor
   Shift + ↑ / ↓   Start selection and extend
   j / k           Extend selection (while in select mode)
   s               Squash selected commits
                   (opens your editor to rewrite the message)
   Esc             Cancel selection

 REORDER (modifies git history, checks for conflicts)
   Ctrl + ↑ / k    Move patch up (toward newer)
   Ctrl + ↓ / j    Move patch down (toward older)

 EDITING (modifies git history)
   e               Edit the commit at cursor
                   (make changes, press Enter — auto stages + amends + rebases)
   i               Insert a new commit (choose: after cursor or at top)
   x               Remove the commit from git history (confirms first)

 STACK OPERATIONS
   r               Rebase entire stack onto base branch
   p               Submit new PR or update existing PR
   s               Sync all submitted PRs (force-push + update bases)
   P               Pull remote changes into local stack
                   (merges teammate's updates, then syncs)
   R               Refresh stack display (re-reads commits from git)
   d               View full diff of commit at cursor

 HISTORY (undo/redo restores actual git state)
   u               Undo last operation
   Ctrl + r        Redo last undone operation
   h               View undo/redo history

 OTHER
   ?               Toggle this help screen
   q               Quit pilegit"
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.notification = Some(msg.into());
    }

    pub fn clear_notification(&mut self) {
        self.notification = None;
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.select_anchor.map(|anchor| {
            (anchor.min(self.cursor), anchor.max(self.cursor))
        })
    }

    /// Get the current git HEAD hash.
    fn current_head() -> Result<String> {
        crate::git::repo_loader::open_resolved().and_then(|r| r.get_head_hash())
    }

    /// Record the current state + HEAD hash in the undo timeline.
    fn record(&mut self, description: &str) {
        let head = Self::current_head().unwrap_or_default();
        self.history.push(description, &self.stack, &head);
    }

    /// Undo: restores git history to the previous state via `git reset --hard`.
    pub fn undo(&mut self) {
        // Check for uncommitted changes before reset --hard
        if let Ok(repo) = crate::git::repo_loader::open_resolved() {
            if repo.has_uncommitted_changes() {
                self.notify("Undo blocked: you have uncommitted changes. Commit or stash first.");
                return;
            }
        }
        if let Some((prev_stack, head_hash)) = self.history.undo() {
            let stack = prev_stack.clone();
            let hash = head_hash.to_string();
            // Reset git to the previous HEAD
            if let Ok(repo) = crate::git::repo_loader::open_resolved() {
                if let Err(e) = repo.reset_hard(&hash) {
                    self.notify(format!("Undo git reset failed: {}", e));
                    return;
                }
            }
            self.stack = stack;
            self.clamp_cursor();
            self.notify(format!("Undone ({}/{})", self.history.position(), self.history.total()));
        } else {
            self.notify("Nothing to undo.");
        }
    }

    /// Redo: advances git history to the next state via `git reset --hard`.
    pub fn redo(&mut self) {
        // Check for uncommitted changes before reset --hard
        if let Ok(repo) = crate::git::repo_loader::open_resolved() {
            if repo.has_uncommitted_changes() {
                self.notify("Redo blocked: you have uncommitted changes. Commit or stash first.");
                return;
            }
        }
        if let Some((next_stack, head_hash)) = self.history.redo() {
            let stack = next_stack.clone();
            let hash = head_hash.to_string();
            if let Ok(repo) = crate::git::repo_loader::open_resolved() {
                if let Err(e) = repo.reset_hard(&hash) {
                    self.notify(format!("Redo git reset failed: {}", e));
                    return;
                }
            }
            self.stack = stack;
            self.clamp_cursor();
            self.notify(format!("Redone ({}/{})", self.history.position(), self.history.total()));
        } else {
            self.notify("Nothing to redo.");
        }
    }

    pub fn clamp_cursor(&mut self) {
        if self.stack.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.stack.len() {
            self.cursor = self.stack.len() - 1;
        }
    }

    // Visual: up = newer = higher index, down = older = lower index

    pub fn move_cursor_up(&mut self) {
        self.clear_notification();
        if !self.stack.is_empty() && self.cursor < self.stack.len() - 1 {
            self.cursor += 1;
        }
    }

    pub fn move_cursor_down(&mut self) {
        self.clear_notification();
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move patch at cursor visually upward by swapping in git history.
    pub fn move_patch_up(&mut self) {
        if self.stack.is_empty() || self.cursor >= self.stack.len() - 1 {
            return;
        }
        let name_below = self.short_desc(self.cursor);
        let hash_below = self.short_hash(self.cursor);
        let hash_above = self.short_hash(self.cursor + 1);
        self.notify("Reordering...");

        match crate::git::repo_loader::open_resolved().and_then(|r| r.swap_commits(&hash_below, &hash_above)) {
            Ok(true) => {
                // Fix dependency trailers after reorder (e.g. "Depends on DXXX" for Phabricator)
                if let Ok(r) = crate::git::repo_loader::open_resolved() {
                    let _ = self.forge.fix_dependencies(&r);
                }
                if let Err(e) = self.reload_stack() {
                    self.notify(format!("Reload failed: {}", e));
                    return;
                }
                self.cursor += 1;
                self.clamp_cursor();
                self.record(&format!("move up: {}", name_below));
                self.notify("Patch moved up.");
            }
            Ok(false) => {
                self.notify("Conflict while reordering.");
                self.wants_suspend = Some(SuspendReason::RebaseConflict);
            }
            Err(e) => self.notify(format!("Reorder failed: {}", e)),
        }
    }

    /// Move patch at cursor visually downward by swapping in git history.
    pub fn move_patch_down(&mut self) {
        if self.cursor == 0 || self.stack.is_empty() {
            return;
        }
        let name_above = self.short_desc(self.cursor);
        let hash_below = self.short_hash(self.cursor - 1);
        let hash_above = self.short_hash(self.cursor);
        self.notify("Reordering...");

        match crate::git::repo_loader::open_resolved().and_then(|r| r.swap_commits(&hash_below, &hash_above)) {
            Ok(true) => {
                // Fix dependency trailers after reorder (e.g. "Depends on DXXX" for Phabricator)
                if let Ok(r) = crate::git::repo_loader::open_resolved() {
                    let _ = self.forge.fix_dependencies(&r);
                }
                if let Err(e) = self.reload_stack() {
                    self.notify(format!("Reload failed: {}", e));
                    return;
                }
                self.cursor -= 1;
                self.clamp_cursor();
                self.record(&format!("move down: {}", name_above));
                self.notify("Patch moved down.");
            }
            Ok(false) => {
                self.notify("Conflict while reordering.");
                self.wants_suspend = Some(SuspendReason::RebaseConflict);
            }
            Err(e) => self.notify(format!("Reorder failed: {}", e)),
        }
    }

    pub fn squash_selected(&mut self) {
        if let Some((lo, hi)) = self.selection_range() {
            let count = hi - lo + 1;
            if count < 2 {
                self.notify("Need at least 2 commits to squash.");
                self.select_anchor = None;
                self.mode = Mode::Normal;
                return;
            }

            // Collect hashes and build a default combined message
            let hashes: Vec<String> = (lo..=hi)
                .map(|i| self.short_hash(i))
                .collect();
            let default_body = (lo..=hi)
                .map(|i| self.stack.patches[i].subject.clone())
                .collect::<Vec<_>>()
                .join("\n");

            // Preserve forge-specific trailers from squashed commits
            let mut trailers = Vec::new();
            for i in lo..=hi {
                trailers.extend(self.forge.get_trailers(&self.stack.patches[i].body));
            }
            trailers.dedup();

            self.select_anchor = None;
            self.mode = Mode::Normal;

            // Suspend TUI: user edits the message, then we perform the git squash
            self.wants_suspend = Some(SuspendReason::SquashCommits {
                hashes,
                default_body,
                trailers,
            });
            self.notify(format!("Squashing {} commits...", count));
        } else {
            self.notify("No selection. Use V or Shift+↑↓ to select first.");
        }
    }

    /// Remove a commit from git history via interactive rebase.
    pub fn drop_at_cursor(&mut self) {
        if self.stack.is_empty() { return; }
        let hash = self.short_hash(self.cursor);
        let subject = self.stack.patches[self.cursor].subject.clone();
        self.notify("Removing...");

        match crate::git::repo_loader::open_resolved().and_then(|r| r.remove_commit(&hash)) {
            Ok(true) => {
                if let Err(e) = self.reload_stack() {
                    self.notify(format!("Reload failed: {}", e));
                    return;
                }
                self.clamp_cursor();
                self.record(&format!("remove: {}", subject));
                self.notify(format!("Removed: {}", subject));
            }
            Ok(false) => {
                self.notify("Conflict while removing commit.");
                self.wants_suspend = Some(SuspendReason::RebaseConflict);
            }
            Err(e) => self.notify(format!("Remove failed: {}", e)),
        }
    }

    /// Show the insert location prompt.
    pub fn show_insert_choice(&mut self) {
        self.clear_notification();
        self.mode = Mode::InsertChoice;
    }

    pub fn insert_at_head(&mut self) {
        self.mode = Mode::Normal;
        self.wants_suspend = Some(SuspendReason::InsertAtHead);
    }

    pub fn insert_after_cursor(&mut self) {
        self.mode = Mode::Normal;
        if self.stack.is_empty() || self.cursor == self.stack.len() - 1 {
            // Cursor at the top = same as inserting at HEAD
            self.insert_at_head();
            return;
        }
        let hash = self.short_hash(self.cursor);
        self.wants_suspend = Some(SuspendReason::InsertAfterCursor { hash });
    }

    pub fn edit_commit_at_cursor(&mut self) {
        if self.stack.is_empty() { return; }
        let hash = self.short_hash(self.cursor);
        self.wants_suspend = Some(SuspendReason::EditCommit { hash });
    }

    /// Reload the stack from git (submitted status marked by forge).
    pub fn reload_stack(&mut self) -> Result<()> {
        let repo = crate::git::repo_loader::open_resolved()?;
        let mut commits = repo.list_stack_commits()?;
        self.forge.mark_submitted(&repo, &mut commits);
        self.stack = Stack::new(self.stack.base.clone(), commits);
        self.clamp_cursor();
        Ok(())
    }

    /// Record a reload in history with a description.
    pub fn record_reload(&mut self, description: &str) {
        self.record(description);
    }

    pub fn start_rebase(&mut self) {
        self.mode = Mode::Confirm {
            prompt: format!("Rebase onto {}? (y/n)", self.stack.base),
            action: PendingAction::Rebase,
        };
    }

    pub fn execute_rebase(&mut self) -> Result<bool> {
        self.wants_suspend = Some(SuspendReason::Rebase);
        Ok(true)
    }

    pub fn continue_rebase(&mut self) -> Result<bool> {
        let repo = crate::git::repo_loader::open_resolved()?;
        match repo.rebase_continue()? {
            true => {
                self.reload_stack()?;
                self.record("rebase completed");
                self.notify("Rebase completed successfully.");
                Ok(true)
            }
            false => {
                let conflicts = repo.conflicted_files().unwrap_or_default();
                self.notify(format!("CONFLICT: {}", conflicts.join(", ")));
                self.wants_suspend = Some(SuspendReason::RebaseConflict);
                Ok(false)
            }
        }
    }

    pub fn abort_rebase(&mut self) -> Result<()> {
        let repo = crate::git::repo_loader::open_resolved()?;
        repo.rebase_abort()?;
        self.reload_stack()?;
        self.record("rebase aborted");
        self.notify("Rebase aborted.");
        Ok(())
    }

    pub fn submit_at_cursor(&mut self) {
        if self.stack.is_empty() { return; }
        let patch = &self.stack.patches[self.cursor];
        let is_already_submitted = patch.status == crate::core::stack::PatchStatus::Submitted;

        let hash = patch.hash.clone();
        let subject = patch.subject.clone();

        if is_already_submitted {
            // Already submitted — suspend TUI to show progress during update
            self.wants_suspend = Some(SuspendReason::UpdatePR {
                hash,
                subject,
                cursor_index: self.cursor,
            });
        } else {
            // New submission — suspend TUI for PR description
            self.wants_suspend = Some(SuspendReason::SubmitCommit {
                hash,
                subject,
                cursor_index: self.cursor,
            });
        }
    }

    /// Sync all submitted PRs — suspends TUI to show progress.
    pub fn sync_all_prs(&mut self) {
        self.wants_suspend = Some(SuspendReason::SyncPRs);
    }

    /// Pull remote changes into local commits — suspends TUI.
    pub fn pull_remote(&mut self) {
        self.wants_suspend = Some(SuspendReason::PullRemote);
    }

    pub fn show_help(&mut self) {
        self.mode = Mode::Help;
    }

    /// Get the short hash (7 chars) for the commit at a given index.
    fn short_hash(&self, index: usize) -> String {
        let h = &self.stack.patches[index].hash;
        h[..7.min(h.len())].to_string()
    }

    /// Get a short description (hash + truncated subject) for history messages.
    fn short_desc(&self, index: usize) -> String {
        let p = &self.stack.patches[index];
        let h = &p.hash[..7.min(p.hash.len())];
        let s = if p.subject.len() > 30 {
            format!("{}...", &p.subject[..27])
        } else {
            p.subject.clone()
        };
        format!("{} {}", h, s)
    }
}
