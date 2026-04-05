use std::collections::HashMap;
use std::process::Command;

use color_eyre::Result;

use super::Forge;
use crate::core::stack::PatchEntry;
use crate::git::ops::Repo;

/// Phabricator uses `arc diff` which doesn't need named branches.
/// Each commit becomes a Differential revision.
pub struct Phabricator;

impl Forge for Phabricator {
    fn name(&self) -> &str { "Phabricator" }
    fn uses_branches(&self) -> bool { false }

    fn submit(
        &self, repo: &Repo, hash: &str, _subject: &str,
        _base: &str, _body: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;

        // Checkout the target commit and run arc diff
        repo.git_pub(&["checkout", "--quiet", hash])?;
        let result = Command::new("arc")
            .current_dir(&repo.workdir)
            .args(["diff", "HEAD^"])
            .output()?;
        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        if result.status.success() {
            Ok(format!("Revision created: {}", stdout.trim()))
        } else {
            Ok(format!("arc diff: {}{}", stdout.trim(), stderr.trim()))
        }
    }

    fn update(
        &self, repo: &Repo, hash: &str, subject: &str,
        base: &str, 
    ) -> Result<String> {
        // For Phabricator, update = re-run arc diff
        self.submit(repo, hash, subject, base, "")
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        // Phabricator doesn't track via branches
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        // Not applicable for Phabricator
        true
    }

    fn mark_submitted(&self, _repo: &Repo, _patches: &mut [PatchEntry]) {
        // Phabricator doesn't use branches — can't auto-detect
    }

    fn sync(
        &self, _repo: &Repo, _patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Phabricator revisions are managed by arc — no sync needed.");
        Ok(Vec::new())
    }
}
