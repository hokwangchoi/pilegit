use serde::{Deserialize, Serialize};

/// A single commit entry in the stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchEntry {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub author: String,
    pub timestamp: String,
    /// The pgit branch name if a PR has been submitted for this commit
    pub pr_branch: Option<String>,
    /// GitHub PR number if submitted
    pub pr_number: Option<u32>,
    /// Current status in the stack
    pub status: PatchStatus,
}

impl PatchEntry {
    /// Create a minimal entry (useful for testing and placeholders).
    pub fn new(hash: &str, subject: &str) -> Self {
        Self {
            hash: hash.to_string(),
            subject: subject.to_string(),
            body: String::new(),
            author: String::new(),
            timestamp: String::new(),
            pr_branch: None,
            pr_number: None,
            status: PatchStatus::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum PatchStatus {
    #[default]
    Clean,
    Conflict,
    Editing,
    Submitted,
    Merged,
}

/// The full stack state — an ordered list of patches from bottom (oldest) to top (newest).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stack {
    /// Base branch this stack is built on (e.g. "main", "origin/main")
    pub base: String,
    /// Ordered patches, index 0 = bottom of stack (oldest)
    pub patches: Vec<PatchEntry>,
}

impl Stack {
    pub fn new(base: String, patches: Vec<PatchEntry>) -> Self {
        Self { base, patches }
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    /// Squash patches at `indices` into the lowest-indexed one.
    pub fn squash(&mut self, indices: &[usize]) -> Result<(), StackError> {
        if indices.is_empty() {
            return Err(StackError::EmptySelection);
        }
        let mut sorted: Vec<usize> = indices.to_vec();
        sorted.sort();
        sorted.dedup();

        // Single commit — nothing to squash
        if sorted.len() == 1 {
            return Ok(());
        }
        if *sorted.last().unwrap() >= self.patches.len() {
            return Err(StackError::OutOfBounds);
        }

        let target_idx = sorted[0];
        let squashed_subjects: Vec<String> = sorted
            .iter()
            .skip(1)
            .map(|&i| self.patches[i].subject.clone())
            .collect();

        // Merge subjects into the target's body
        let addendum = squashed_subjects.join("\n");
        if self.patches[target_idx].body.is_empty() {
            self.patches[target_idx].body = addendum;
        } else {
            self.patches[target_idx].body.push('\n');
            self.patches[target_idx].body.push_str(&addendum);
        }
        self.patches[target_idx].subject = format!(
            "{} (+{} squashed)",
            self.patches[target_idx].subject,
            squashed_subjects.len()
        );

        // Remove squashed entries highest-index-first to keep indices stable
        for &i in sorted.iter().skip(1).rev() {
            self.patches.remove(i);
        }
        Ok(())
    }

    /// Move a patch from `from` to `to` position.
    pub fn reorder(&mut self, from: usize, to: usize) -> Result<(), StackError> {
        if from >= self.patches.len() || to >= self.patches.len() {
            return Err(StackError::OutOfBounds);
        }
        if from == to {
            return Ok(());
        }
        let entry = self.patches.remove(from);
        self.patches.insert(to, entry);
        Ok(())
    }

    /// Insert a new patch at the given position.
    pub fn insert_at(&mut self, index: usize, entry: PatchEntry) -> Result<(), StackError> {
        if index > self.patches.len() {
            return Err(StackError::OutOfBounds);
        }
        self.patches.insert(index, entry);
        Ok(())
    }

    /// Drop a patch from the stack.
    pub fn drop_patch(&mut self, index: usize) -> Result<PatchEntry, StackError> {
        if index >= self.patches.len() {
            return Err(StackError::OutOfBounds);
        }
        Ok(self.patches.remove(index))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StackError {
    #[error("no patches selected")]
    EmptySelection,
    #[error("index out of bounds")]
    OutOfBounds,
}

