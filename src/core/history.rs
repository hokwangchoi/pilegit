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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stack::{PatchEntry, PatchStatus, Stack};

    fn dummy_stack(n: usize) -> Stack {
        let patches: Vec<PatchEntry> = (0..n)
            .map(|i| PatchEntry {
                hash: format!("abc{}", i),
                subject: format!("commit {}", i),
                body: String::new(),
                author: "test".into(),
                timestamp: "2026-01-01".into(),
                pr_branch: None,
                pr_number: None,
                pr_url: None,
                status: PatchStatus::Clean,
            })
            .collect();
        Stack::new("main".into(), patches)
    }

    #[test]
    fn push_and_undo() {
        let mut h = History::new(100);
        h.push("initial", &dummy_stack(1), "aaa");
        h.push("second", &dummy_stack(2), "bbb");
        h.push("third", &dummy_stack(3), "ccc");

        assert_eq!(h.total(), 3);
        assert_eq!(h.position(), 2);

        let (stack, hash) = h.undo().unwrap();
        assert_eq!(stack.len(), 2);
        assert_eq!(hash, "bbb");

        let (stack, hash) = h.undo().unwrap();
        assert_eq!(stack.len(), 1);
        assert_eq!(hash, "aaa");

        // Can't undo past the beginning
        assert!(h.undo().is_none());
    }

    #[test]
    fn redo_after_undo() {
        let mut h = History::new(100);
        h.push("a", &dummy_stack(1), "h1");
        h.push("b", &dummy_stack(2), "h2");
        h.push("c", &dummy_stack(3), "h3");

        h.undo(); // → b
        h.undo(); // → a

        let (stack, hash) = h.redo().unwrap();
        assert_eq!(stack.len(), 2);
        assert_eq!(hash, "h2");

        let (stack, _) = h.redo().unwrap();
        assert_eq!(stack.len(), 3);

        // Can't redo past the end
        assert!(h.redo().is_none());
    }

    #[test]
    fn push_after_undo_clears_redo() {
        let mut h = History::new(100);
        h.push("a", &dummy_stack(1), "h1");
        h.push("b", &dummy_stack(2), "h2");
        h.push("c", &dummy_stack(3), "h3");

        h.undo(); // → b
        h.push("d", &dummy_stack(4), "h4"); // replaces c

        assert_eq!(h.total(), 3); // a, b, d
        assert!(h.redo().is_none()); // c is gone
    }

    #[test]
    fn max_entries_eviction() {
        let mut h = History::new(3);
        h.push("a", &dummy_stack(1), "h1");
        h.push("b", &dummy_stack(2), "h2");
        h.push("c", &dummy_stack(3), "h3");
        h.push("d", &dummy_stack(4), "h4"); // evicts a

        assert_eq!(h.total(), 3); // b, c, d
        let descriptions: Vec<&str> = h.list().iter().map(|e| e.description.as_str()).collect();
        assert_eq!(descriptions, vec!["b", "c", "d"]);
    }

    #[test]
    fn empty_history() {
        let mut h = History::new(10);
        assert!(h.undo().is_none());
        assert!(h.redo().is_none());
        assert!(!h.can_undo());
        assert_eq!(h.total(), 0);
    }
}
