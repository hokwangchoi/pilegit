use super::stack::Stack;

/// A snapshot of the stack state at a point in time,
/// along with the git HEAD hash so undo/redo can restore git history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub description: String,
    pub snapshot: Stack,
    /// The git HEAD commit hash at this point in time.
    /// Used by undo/redo to `git reset --hard` back to this state.
    pub head_hash: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Maintains an undo/redo history as a timeline of stack states.
///
/// entries[0] is the initial state, entries[cursor] is the current state.
/// Undo decrements cursor, redo increments it.
pub struct History {
    entries: Vec<HistoryEntry>,
    /// Index of the current state in entries.
    cursor: usize,
    max_entries: usize,
}

impl History {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_entries,
        }
    }

    /// Record a new state after an operation.
    /// `head_hash` is the current git HEAD so we can restore it on undo.
    pub fn push(
        &mut self,
        description: impl Into<String>,
        stack: &Stack,
        head_hash: impl Into<String>,
    ) {
        // If cursor isn't at the end, discard redo-able future
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(HistoryEntry {
            description: description.into(),
            snapshot: stack.clone(),
            head_hash: head_hash.into(),
            timestamp: chrono::Utc::now(),
        });
        self.cursor = self.entries.len() - 1;

        // Enforce max entries by evicting from the front
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
            self.cursor = self.entries.len() - 1;
        }
    }

    /// Undo: move to the previous state.
    /// Returns (stack, head_hash) or None if at the beginning.
    pub fn undo(&mut self) -> Option<(&Stack, &str)> {
        if self.cursor == 0 || self.entries.is_empty() {
            return None;
        }
        self.cursor -= 1;
        let entry = &self.entries[self.cursor];
        Some((&entry.snapshot, &entry.head_hash))
    }

    /// Redo: move to the next state.
    /// Returns (stack, head_hash) or None if at the end.
    pub fn redo(&mut self) -> Option<(&Stack, &str)> {
        if self.entries.is_empty() || self.cursor >= self.entries.len() - 1 {
            return None;
        }
        self.cursor += 1;
        let entry = &self.entries[self.cursor];
        Some((&entry.snapshot, &entry.head_hash))
    }

    /// List all history entries for display.
    pub fn list(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn position(&self) -> usize {
        self.cursor
    }

    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.entries.is_empty() && self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        !self.entries.is_empty() && self.cursor < self.entries.len() - 1
    }
}
