use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

/// Phabricator integration via `arc` CLI.
///
/// Key insight: `arc diff` amends the commit to add a `Differential Revision:`
/// trailer. To keep this trailer in the branch history, submit/update use
/// interactive rebase (`edit` marker) so the amendment happens in-place.
pub struct Phabricator;

impl Forge for Phabricator {
    fn name(&self) -> &str { "Phabricator" }
    fn needs_description_editor(&self) -> bool { false }

    fn get_trailers(&self, body: &str) -> Vec<String> {
        body.lines()
            .filter(|l| l.trim().starts_with("Differential Revision:"))
            .map(|l| l.trim().to_string())
            .collect()
    }

    fn submit(
        &self, repo: &Repo, hash: &str, _subject: &str,
        _base: &str, _body: &str,
    ) -> Result<String> {
        let short = &hash[..7.min(hash.len())];

        // Pause rebase at the target commit
        match repo.rebase_edit_commit(short) {
            Ok(false) => {} // paused — good
            Ok(true) => return Err(eyre!("Commit {} not found in stack", short)),
            Err(e) => return Err(eyre!("Failed to start rebase: {}", e)),
        }

        // Run arc diff interactively with full terminal access.
        let status = Command::new("arc")
            .current_dir(&repo.workdir)
            .args(["diff", "HEAD^"])
            .status();

        // Parse revision ID from the (possibly amended) commit message
        let msg = repo.git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        // Continue rebase to replay the rest of the stack
        let rebase_ok = match repo.rebase_continue() {
            Ok(true) => true,
            Ok(false) => false, // conflicts
            Err(_) => false,
        };

        // A revision ID in the commit trailer is the strongest signal of success.
        // Arc sometimes exits non-zero even on success (e.g. lint warnings).
        let arc_ran = status.is_ok();
        let exit_ok = status.map(|s| s.success()).unwrap_or(false);

        if revision_id.is_some() || exit_ok {
            let id_str = revision_id
                .map(|id| format!("D{}", id))
                .unwrap_or_else(|| "unknown".to_string());
            if rebase_ok {
                Ok(format!("Revision created: {}", id_str))
            } else {
                Ok(format!("Revision created: {} (rebase has conflicts — resolve and run `git rebase --continue`)", id_str))
            }
        } else if !arc_ran {
            let _ = repo.rebase_abort();
            Err(eyre!("arc not found — install arcanist"))
        } else {
            let _ = repo.rebase_abort();
            Err(eyre!("arc diff failed"))
        }
    }

    fn update(
        &self, repo: &Repo, hash: &str, _subject: &str, _base: &str,
    ) -> Result<String> {
        let short = &hash[..7.min(hash.len())];

        // Get revision ID from the commit message before rebasing
        let msg = repo.git_pub(&["log", "-1", "--format=%B", hash])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        // Pause rebase at the target commit
        match repo.rebase_edit_commit(short) {
            Ok(false) => {}
            Ok(true) => return Err(eyre!("Commit {} not found in stack", short)),
            Err(e) => return Err(eyre!("Failed to start rebase: {}", e)),
        }

        // Run arc diff to update the existing revision
        let status = match &revision_id {
            Some(id) => {
                Command::new("arc")
                    .current_dir(&repo.workdir)
                    .args(["diff", "HEAD^", "--update", &format!("D{}", id)])
                    .status()
            }
            None => {
                Command::new("arc")
                    .current_dir(&repo.workdir)
                    .args(["diff", "HEAD^"])
                    .status()
            }
        };

        // Re-parse revision ID from commit message (arc may have amended it)
        let msg = repo.git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg).or(revision_id);

        // Continue rebase
        let rebase_ok = match repo.rebase_continue() {
            Ok(true) => true,
            Ok(false) => false,
            Err(_) => false,
        };

        let arc_ran = status.is_ok();
        let exit_ok = status.map(|s| s.success()).unwrap_or(false);

        if revision_id.is_some() || exit_ok {
            let id_str = revision_id
                .map(|id| format!("D{}", id))
                .unwrap_or_else(|| "unknown".to_string());
            if rebase_ok {
                Ok(format!("Revision updated: {}", id_str))
            } else {
                Ok(format!("Revision updated: {} (rebase has conflicts — resolve and run `git rebase --continue`)", id_str))
            }
        } else if !arc_ran {
            let _ = repo.rebase_abort();
            Err(eyre!("arc not found — install arcanist"))
        } else {
            let _ = repo.rebase_abort();
            Err(eyre!("arc diff failed"))
        }
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        true
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        // Scan commit messages for "Differential Revision:" trailers.
        // These trailers are added by arc diff during submit and preserved
        // through rebases, so they persist in the branch history.
        for patch in patches.iter_mut() {
            let full = repo.git_pub(&["log", "-1", "--format=%B", &patch.hash])
                .unwrap_or_default();
            if let Some((id, url)) = parse_revision_id_and_url(&full) {
                patch.status = PatchStatus::Submitted;
                patch.pr_number = Some(id);
                patch.pr_url = Some(url);
            }
        }
    }

    fn sync(
        &self, repo: &Repo, patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        // For sync, we use detached HEAD since we just need to upload the
        // latest diff — the Differential Revision trailer is already in the
        // commit from the initial submit. arc detects it automatically.
        let branch = repo.get_current_branch()?;
        let mut updates = Vec::new();

        for patch in patches {
            if patch.status != PatchStatus::Submitted { continue; }
            let id = match patch.pr_number {
                Some(id) => id,
                None => continue,
            };

            on_progress(&format!("Updating D{}: {} ...", id, &patch.subject));

            if repo.git_pub(&["checkout", "--quiet", &patch.hash]).is_err() {
                updates.push(format!("⚠ D{} checkout failed, skipping", id));
                continue;
            }

            // Non-interactive: --message provides update comment (no editor),
            // stdin null prevents any remaining prompts.
            // Note: --verbatim and --update are mutually exclusive in arc.
            let status = Command::new("arc")
                .current_dir(&repo.workdir)
                .args(["diff", "HEAD^", "--update", &format!("D{}", id),
                    "--message", "Updated diff"])
                .stdin(std::process::Stdio::null())
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
    parse_revision_id_and_url(message).map(|(id, _)| id)
}

/// Parse both revision ID and URL from a commit message.
/// Returns (id, url) where url is the full "Differential Revision:" value.
fn parse_revision_id_and_url(message: &str) -> Option<(u32, String)> {
    for line in message.lines() {
        let line = line.trim();
        if line.starts_with("Differential Revision:") {
            let url_part = line.trim_start_matches("Differential Revision:").trim();
            if let Some(d_pos) = line.rfind('D') {
                let num_str: String = line[d_pos + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(id) = num_str.parse::<u32>() {
                    return Some((id, url_part.to_string()));
                }
            }
        }
    }
    None
}

/// Parse a Phabricator revision ID from arc's stdout output.
/// Looks for "Revision URI: .../DXXXX" patterns in arc's output.
#[allow(dead_code)]
fn parse_revision_from_arc_output(output: &str) -> Option<u32> {
    for line in output.lines() {
        let line = line.trim();
        // Match "Revision URI: https://phab.example.com/D1234"
        if line.contains("Revision URI:") || line.contains("revision/") {
            if let Some(d_pos) = line.rfind("/D") {
                let num_str: String = line[d_pos + 2..]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_revision_url() {
        let msg = "Some commit\n\nDifferential Revision: https://phab.example.com/D1234";
        assert_eq!(parse_revision_id(msg), Some(1234));
        let (id, url) = parse_revision_id_and_url(msg).unwrap();
        assert_eq!(id, 1234);
        assert_eq!(url, "https://phab.example.com/D1234");
    }

    #[test]
    fn parse_revision_bare() {
        assert_eq!(parse_revision_id("Differential Revision: D5678"), Some(5678));
        let (id, url) = parse_revision_id_and_url("Differential Revision: D5678").unwrap();
        assert_eq!(id, 5678);
        assert_eq!(url, "D5678");
    }

    #[test]
    fn parse_revision_multiline() {
        let msg = "fix bug\n\nDifferential Revision: https://phab.co/D42\nSome other line";
        assert_eq!(parse_revision_id(msg), Some(42));
    }

    #[test]
    fn parse_revision_not_present() {
        assert_eq!(parse_revision_id("just a commit message"), None);
        assert_eq!(parse_revision_id(""), None);
    }

    #[test]
    fn parse_revision_wrong_prefix() {
        assert_eq!(parse_revision_id("Reviewed-by: D9999"), None);
    }

    #[test]
    fn parse_arc_output_revision_uri() {
        let output = "Updated an existing Differential revision:\n        Revision URI: https://p.daedalean.ai/D32750\n\nIncluded changes:\n  M  file.rs";
        assert_eq!(parse_revision_from_arc_output(output), Some(32750));
    }

    #[test]
    fn parse_arc_output_created() {
        let output = "Created a new Differential revision:\n        Revision URI: https://phab.example.com/D999";
        assert_eq!(parse_revision_from_arc_output(output), Some(999));
    }

    #[test]
    fn parse_arc_output_no_revision() {
        assert_eq!(parse_revision_from_arc_output("Linting...\n OKAY"), None);
        assert_eq!(parse_revision_from_arc_output(""), None);
    }
}
