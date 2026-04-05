use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

/// Phabricator integration via `arc` CLI.
///
/// Workflow:
/// - Submit: `arc diff HEAD^` — creates a Differential revision
/// - Update: `arc diff HEAD^ --update DXXX` — updates an existing revision
/// - Detection: parses `Differential Revision:` trailer from commit messages
pub struct Phabricator;

impl Forge for Phabricator {
    fn name(&self) -> &str { "Phabricator" }
    fn uses_branches(&self) -> bool { false }
    fn needs_description_editor(&self) -> bool { false }

    fn submit(
        &self, repo: &Repo, hash: &str, _subject: &str,
        _base: &str, _body: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;

        repo.git_pub(&["checkout", "--quiet", hash])?;

        // Run arc diff interactively — user writes the revision description
        let status = Command::new("arc")
            .current_dir(&repo.workdir)
            .args(["diff", "HEAD^"])
            .status()?;

        // After arc finishes, check if it added a Differential Revision trailer
        let msg = repo.git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        if status.success() {
            match revision_id {
                Some(id) => Ok(format!("Revision created: D{}", id)),
                None => Ok("Revision created (could not parse ID — add Differential Revision: trailer to commit)".to_string()),
            }
        } else {
            Err(eyre!("arc diff failed"))
        }
    }

    fn update(
        &self, repo: &Repo, hash: &str, _subject: &str, _base: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;

        repo.git_pub(&["checkout", "--quiet", hash])?;

        // Try to get revision ID from commit message
        let msg = repo.git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        let status = match &revision_id {
            Some(id) => {
                // Update existing revision
                Command::new("arc")
                    .current_dir(&repo.workdir)
                    .args(["diff", "HEAD^", "--update", &format!("D{}", id)])
                    .status()?
            }
            None => {
                // No revision ID — let arc try to figure it out
                Command::new("arc")
                    .current_dir(&repo.workdir)
                    .args(["diff", "HEAD^"])
                    .status()?
            }
        };

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        if status.success() {
            match revision_id {
                Some(id) => Ok(format!("Revision updated: D{}", id)),
                None => Ok("Revision updated".to_string()),
            }
        } else {
            Err(eyre!("arc diff failed"))
        }
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        // Phabricator doesn't track via branches
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        true // Not applicable
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        // Scan commit messages for "Differential Revision:" trailers
        for patch in patches.iter_mut() {
            if let Some(id) = parse_revision_id(&patch.body) {
                patch.status = PatchStatus::Submitted;
                patch.pr_number = Some(id);
            } else {
                // Also check the full commit message (subject + body)
                let full = repo.git_pub(&["log", "-1", "--format=%B", &patch.hash])
                    .unwrap_or_default();
                if let Some(id) = parse_revision_id(&full) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_number = Some(id);
                }
            }
        }
    }

    fn sync(
        &self, repo: &Repo, patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        let branch = repo.get_current_branch()?;
        let mut updates = Vec::new();

        for patch in patches {
            if patch.status != PatchStatus::Submitted { continue; }
            let id = match patch.pr_number {
                Some(id) => id,
                None => continue,
            };

            on_progress(&format!("Updating D{}: {} ...", id, &patch.subject));

            repo.git_pub(&["checkout", "--quiet", &patch.hash])?;
            let status = Command::new("arc")
                .current_dir(&repo.workdir)
                .args(["diff", "HEAD^", "--update", &format!("D{}", id)])
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();

            match status {
                Ok(s) if s.success() => {
                    updates.push(format!("✓ D{} updated", id));
                }
                _ => {
                    updates.push(format!("⚠ D{} update failed", id));
                }
            }
        }

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);
        Ok(updates)
    }
}

/// Parse a Phabricator revision ID from a commit message.
/// Looks for "Differential Revision: .../DXXXX" or just "D" followed by digits.
fn parse_revision_id(message: &str) -> Option<u32> {
    for line in message.lines() {
        let line = line.trim();
        if line.starts_with("Differential Revision:") {
            // Extract DXXXX from URL or bare reference
            if let Some(d_pos) = line.rfind('D') {
                let num_str: String = line[d_pos + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(id) = num_str.parse::<u32>() {
                    return Some(id);
                }
            }
        }
    }
    None
}
