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
}
