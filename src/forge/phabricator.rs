use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::{sed_inplace_shell_prefix, Repo};

/// Phabricator integration via `arc` CLI.
///
/// Key insight: `arc diff` amends the commit to add a `Differential Revision:`
/// trailer. To keep this trailer in the branch history, submit/update use
/// interactive rebase (`edit` marker) so the amendment happens in-place.
pub struct Phabricator;

impl Forge for Phabricator {
    fn name(&self) -> &str {
        "Phabricator"
    }
    fn needs_description_editor(&self) -> bool {
        false
    }

    fn get_trailers(&self, body: &str) -> Vec<String> {
        body.lines()
            .filter(|l| l.trim().starts_with("Differential Revision:"))
            .map(|l| l.trim().to_string())
            .collect()
    }

    fn submit(
        &self,
        repo: &Repo,
        hash: &str,
        subject: &str,
        _base: &str,
        _body: &str,
    ) -> Result<String> {
        let short = &hash[..7.min(hash.len())];
        let branch_name = repo.make_pgit_branch_name(subject);

        // Pause rebase at the target commit
        match repo.rebase_edit_commit(short) {
            Ok(false) => {} // paused — good
            Ok(true) => return Err(eyre!("Commit {} not found in stack", short)),
            Err(e) => return Err(eyre!("Failed to start rebase: {}", e)),
        }

        // Check if the parent commit (HEAD^) has a Differential Revision trailer
        // AND is still in the stack (not already landed in the base branch).
        // If the parent is in base, its diff is already landed — no dependency needed.
        let repo_base = repo.base().unwrap_or_else(|_| "origin/main".to_string());
        let parent_in_base = repo
            .git_pub(&["merge-base", "--is-ancestor", "HEAD^", &repo_base])
            .is_ok();
        if !parent_in_base {
            let parent_msg = repo
                .git_pub(&["log", "-1", "--format=%B", "HEAD^"])
                .unwrap_or_default();
            if let Some(parent_id) = parse_revision_id(&parent_msg) {
                let current_msg = repo
                    .git_pub(&["log", "-1", "--format=%B"])
                    .unwrap_or_default();
                let current_trimmed = current_msg.trim();

                // Remove any existing Depends on line, then add the correct one
                let without_depends: String = current_trimmed
                    .lines()
                    .filter(|l| !l.trim().starts_with("Depends on D"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let new_msg = format!("{}\n\nDepends on D{}", without_depends.trim(), parent_id);

                if new_msg != current_trimmed {
                    let _ = Command::new("git")
                        .current_dir(&repo.workdir)
                        .args(["commit", "--amend", "--message", &new_msg])
                        .output();
                }
            }
        }

        // Run arc diff interactively with full terminal access.
        let status = Command::new("arc")
            .current_dir(&repo.workdir)
            .args(["diff", "HEAD^"])
            .status();

        // Parse revision ID from the (possibly amended) commit message
        let msg = repo
            .git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        // Capture the amended commit hash before continuing rebase
        let amended_hash = repo
            .git_pub(&["rev-parse", "HEAD"])
            .unwrap_or_default()
            .trim()
            .to_string();

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
            // Create and push a pgit branch so CI/CD (e.g. Drone) can detect the commit
            if rebase_ok {
                // After rebase, find the new hash by matching the Differential Revision trailer
                if let Ok(new_hash) = find_commit_with_revision(repo, revision_id) {
                    let _ = repo.git_pub(&["branch", "-f", &branch_name, &new_hash]);
                    let _ = repo.git_pub(&["push", "-f", "origin", &branch_name]);
                }
            } else if !amended_hash.is_empty() {
                let _ = repo.git_pub(&["branch", "-f", &branch_name, &amended_hash]);
                let _ = repo.git_pub(&["push", "-f", "origin", &branch_name]);
            }

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

    fn update(&self, repo: &Repo, hash: &str, subject: &str, _base: &str) -> Result<String> {
        let short = &hash[..7.min(hash.len())];
        let branch_name = repo.make_pgit_branch_name(subject);

        // Get revision ID from the commit message before rebasing
        let msg = repo
            .git_pub(&["log", "-1", "--format=%B", hash])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg);

        // Pause rebase at the target commit
        match repo.rebase_edit_commit(short) {
            Ok(false) => {}
            Ok(true) => return Err(eyre!("Commit {} not found in stack", short)),
            Err(e) => return Err(eyre!("Failed to start rebase: {}", e)),
        }

        // Update "Depends on DXXX" based on current parent commit.
        // Skip if parent is already in the base branch (diff already landed).
        let base = repo.base().unwrap_or_else(|_| "origin/main".to_string());
        let parent_in_base = repo
            .git_pub(&["merge-base", "--is-ancestor", "HEAD^", &base])
            .is_ok();

        let current_msg = repo
            .git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let current_trimmed = current_msg.trim();

        // Remove any existing Depends on line
        let without_depends: String = current_trimmed
            .lines()
            .filter(|l| !l.trim().starts_with("Depends on D"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut new_msg = without_depends.trim().to_string();

        // Add new Depends on only if parent is in the stack (not in base)
        if !parent_in_base {
            let parent_msg = repo
                .git_pub(&["log", "-1", "--format=%B", "HEAD^"])
                .unwrap_or_default();
            if let Some(parent_id) = parse_revision_id(&parent_msg) {
                new_msg.push_str(&format!("\n\nDepends on D{}", parent_id));
            }
        }

        if new_msg != current_trimmed {
            let _ = Command::new("git")
                .current_dir(&repo.workdir)
                .args(["commit", "--amend", "--message", &new_msg])
                .output();
        }

        // Run arc diff to update the existing revision
        let status = match &revision_id {
            Some(id) => Command::new("arc")
                .current_dir(&repo.workdir)
                .args(["diff", "HEAD^", "--update", &format!("D{}", id)])
                .status(),
            None => Command::new("arc")
                .current_dir(&repo.workdir)
                .args(["diff", "HEAD^"])
                .status(),
        };

        // Re-parse revision ID from commit message (arc may have amended it)
        let msg = repo
            .git_pub(&["log", "-1", "--format=%B"])
            .unwrap_or_default();
        let revision_id = parse_revision_id(&msg).or(revision_id);

        // Capture the amended commit hash before continuing rebase
        let amended_hash = repo
            .git_pub(&["rev-parse", "HEAD"])
            .unwrap_or_default()
            .trim()
            .to_string();

        // Continue rebase
        let rebase_ok = match repo.rebase_continue() {
            Ok(true) => true,
            Ok(false) => false,
            Err(_) => false,
        };

        let arc_ran = status.is_ok();
        let exit_ok = status.map(|s| s.success()).unwrap_or(false);

        if revision_id.is_some() || exit_ok {
            // Update the pgit branch so CI/CD sees the new diff
            if rebase_ok {
                if let Ok(new_hash) = find_commit_with_revision(repo, revision_id) {
                    let _ = repo.git_pub(&["branch", "-f", &branch_name, &new_hash]);
                    let _ = repo.git_pub(&["push", "-f", "origin", &branch_name]);
                }
            } else if !amended_hash.is_empty() {
                let _ = repo.git_pub(&["branch", "-f", &branch_name, &amended_hash]);
                let _ = repo.git_pub(&["push", "-f", "origin", &branch_name]);
            }

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

    fn fix_dependencies(&self, repo: &Repo) -> Result<()> {
        let base = repo.base()?;

        // Check if any commits need Depends on updates
        let commits = repo.list_stack_commits()?;
        let mut needs_fix = false;
        for (i, commit) in commits.iter().enumerate() {
            let msg = repo
                .git_pub(&["log", "-1", "--format=%B", &commit.hash])
                .unwrap_or_default();

            // Only care about submitted commits (have Differential Revision trailer)
            if parse_revision_id(&msg).is_none() {
                continue;
            }

            // Check if Depends on matches the parent's revision
            let current_depends = msg
                .lines()
                .find(|l| l.trim().starts_with("Depends on D"))
                .and_then(|l| {
                    let d_pos = l.find('D')?;
                    l[d_pos + 1..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>()
                        .ok()
                });

            // Find parent's revision ID, but only if parent is in the stack
            // (not in the base branch). If parent is in base, no dependency needed.
            let parent_rev = if i > 0 {
                let parent_msg = repo
                    .git_pub(&["log", "-1", "--format=%B", &commits[i - 1].hash])
                    .unwrap_or_default();
                parse_revision_id(&parent_msg)
            } else {
                // First commit in stack — parent is base, no dependency
                None
            };

            if current_depends != parent_rev {
                needs_fix = true;
                break;
            }
        }

        if !needs_fix {
            return Ok(());
        }

        // Mark all commits as "edit" and rebase to amend each one
        let _ = Command::new("git")
            .current_dir(&repo.workdir)
            .args(["rebase", "-i", &base])
            .env(
                "GIT_SEQUENCE_EDITOR",
                format!("{} 's/^pick /edit /'", sed_inplace_shell_prefix()),
            )
            .output();

        if !repo.is_rebase_in_progress() {
            return Ok(()); // nothing to do or rebase not needed
        }

        // Loop through each paused commit, fix Depends on, continue
        loop {
            if !repo.is_rebase_in_progress() {
                break;
            }

            let current_msg = repo
                .git_pub(&["log", "-1", "--format=%B"])
                .unwrap_or_default();
            let current_trimmed = current_msg.trim();

            // Only amend submitted commits (have Differential Revision trailer)
            if parse_revision_id(current_trimmed).is_some() {
                // Check if parent is in the base branch (already landed)
                let parent_in_base = repo
                    .git_pub(&["merge-base", "--is-ancestor", "HEAD^", &base])
                    .is_ok();

                // Remove existing Depends on
                let without_depends: String = current_trimmed
                    .lines()
                    .filter(|l| !l.trim().starts_with("Depends on D"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut new_msg = without_depends.trim().to_string();

                // Add correct Depends on only if parent is in the stack
                if !parent_in_base {
                    let parent_msg = repo
                        .git_pub(&["log", "-1", "--format=%B", "HEAD^"])
                        .unwrap_or_default();
                    if let Some(parent_id) = parse_revision_id(&parent_msg) {
                        new_msg.push_str(&format!("\n\nDepends on D{}", parent_id));
                    }
                }

                if new_msg != current_trimmed {
                    let _ = Command::new("git")
                        .current_dir(&repo.workdir)
                        .args(["commit", "--amend", "--message", &new_msg])
                        .output();
                }
            }

            // Continue to next commit
            match repo.rebase_continue() {
                Ok(true) => break, // rebase completed
                Ok(false) => {
                    // Paused at next edit commit — continue loop
                    if !repo.is_rebase_in_progress() {
                        break; // rebase done
                    }
                }
                Err(_) => {
                    let _ = repo.rebase_abort();
                    break;
                }
            }
        }

        Ok(())
    }

    fn find_landed_branches(&self, repo: &Repo, branches: &[String]) -> Vec<String> {
        // arc land squashes commits into a new hash but preserves the
        // "Differential Revision:" trailer in the landed commit. For each
        // branch, parse its trailer and search the base branch for a matching one.
        let base = repo.base().unwrap_or_else(|_| "origin/main".to_string());
        let mut landed = Vec::new();
        for b in branches {
            let msg = repo
                .git_pub(&["log", "-1", "--format=%B", b])
                .unwrap_or_default();
            for line in msg.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Differential Revision:") {
                    let trailer = rest.trim();
                    let pattern = format!("Differential Revision: {}", trailer);
                    let found = repo
                        .git_pub(&["log", &base, "--grep", &pattern, "-1", "--format=%H"])
                        .unwrap_or_default();
                    if !found.trim().is_empty() {
                        landed.push(b.clone());
                        break;
                    }
                }
            }
        }
        landed
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        // Scan commit messages for "Differential Revision:" trailers.
        // These trailers are added by arc diff during submit and preserved
        // through rebases, so they persist in the branch history.
        for patch in patches.iter_mut() {
            let full = repo
                .git_pub(&["log", "-1", "--format=%B", &patch.hash])
                .unwrap_or_default();
            if let Some((id, url)) = parse_revision_id_and_url(&full) {
                patch.status = PatchStatus::Submitted;
                patch.pr_number = Some(id);
                patch.pr_url = Some(url);
            }
        }
    }

    fn sync(
        &self,
        repo: &Repo,
        patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        // For sync, we use detached HEAD since we just need to upload the
        // latest diff — the Differential Revision trailer is already in the
        // commit from the initial submit. arc detects it automatically.
        let branch = repo.get_current_branch()?;
        let mut updates = Vec::new();

        // Build a map of patch index → revision ID for dependency lookup
        let submitted: Vec<(usize, u32)> = patches
            .iter()
            .enumerate()
            .filter(|(_, p)| p.status == PatchStatus::Submitted && p.pr_number.is_some())
            .map(|(i, p)| (i, p.pr_number.unwrap()))
            .collect();

        for &(idx, id) in &submitted {
            on_progress(&format!("Updating D{}: {} ...", id, &patches[idx].subject));

            if repo
                .git_pub(&["checkout", "--quiet", &patches[idx].hash])
                .is_err()
            {
                updates.push(format!("⚠ D{} checkout failed, skipping", id));
                continue;
            }

            // Add/update "Depends on DXXX" based on the parent's revision ID
            let parent_rev = find_parent_revision(patches, idx);
            let current_msg = repo
                .git_pub(&["log", "-1", "--format=%B"])
                .unwrap_or_default();
            let current_trimmed = current_msg.trim();

            // Remove existing Depends on, then add correct one
            let without_depends: String = current_trimmed
                .lines()
                .filter(|l| !l.trim().starts_with("Depends on D"))
                .collect::<Vec<_>>()
                .join("\n");
            let mut new_msg = without_depends.trim().to_string();
            if let Some(parent_id) = parent_rev {
                new_msg.push_str(&format!("\n\nDepends on D{}", parent_id));
            }

            if new_msg != current_trimmed {
                let _ = Command::new("git")
                    .current_dir(&repo.workdir)
                    .args(["commit", "--amend", "--message", &new_msg])
                    .output();
            }

            // Run arc diff to update both the code diff and the revision description.
            // arc reads the commit message (with our updated "Depends on DXXX")
            // and uses it to update the revision description automatically.
            // --message sets the update comment (without opening an editor).
            // stdin null prevents interactive prompts.
            let status = Command::new("arc")
                .current_dir(&repo.workdir)
                .args([
                    "diff",
                    "HEAD^",
                    "--update",
                    &format!("D{}", id),
                    "--message",
                    "Updated diff",
                ])
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

            // Push pgit branch so CI/CD (e.g. Drone) sees the update
            let branch_name = repo.make_pgit_branch_name(&patches[idx].subject);
            let _ = repo.git_pub(&["branch", "-f", &branch_name, "HEAD"]);
            let _ = repo.git_pub(&["push", "-f", "origin", &branch_name]);
        }

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);
        Ok(updates)
    }

    fn check_diverged(&self, repo: &Repo, patches: &[PatchEntry]) -> Vec<(String, String)> {
        let mut diverged = Vec::new();
        let saved = repo.read_sync_state();

        for patch in patches {
            if patch.status != PatchStatus::Submitted {
                continue;
            }
            let id = match patch.pr_number {
                Some(id) => id,
                None => continue,
            };

            // Check Phabricator diffPHID (catches arc diff --update by others)
            let current_phid = match get_latest_diff_phid(repo, id) {
                Some(p) => p,
                None => continue,
            };
            let key = format!("D{}", id);
            if let Some(saved_phid) = saved.get(&key) {
                if saved_phid != &current_phid {
                    diverged.push((
                        key,
                        format!("D{} has been updated remotely ('{}')", id, patch.subject),
                    ));
                }
            }
        }

        diverged
    }

    fn get_remote_ref(&self, repo: &Repo, patch: &PatchEntry) -> Option<String> {
        let id = patch.pr_number?;
        let temp_branch = format!("pgit-temp-patch-D{}", id);

        // Create temp branch from base
        let base = repo.base().ok()?;
        let _ = repo.git_pub(&["branch", "-f", &temp_branch, &base]);
        let _ = repo.git_pub(&["checkout", "--quiet", &temp_branch]);

        // Apply the latest diff from Phabricator
        let status = Command::new("arc")
            .current_dir(&repo.workdir)
            .args([
                "patch",
                "--revision",
                &format!("D{}", id),
                "--nobranch",
                "--force",
            ])
            .stdin(std::process::Stdio::null())
            .status();

        match status {
            Ok(s) if s.success() => Some(temp_branch),
            _ => {
                let _ = repo.git_pub(&["checkout", "--quiet", "-"]);
                let _ = repo.git_pub(&["branch", "-D", &temp_branch]);
                None
            }
        }
    }

    fn save_sync_state(&self, repo: &Repo, patches: &[PatchEntry]) {
        let mut state = repo.read_sync_state();
        for patch in patches {
            if patch.status != PatchStatus::Submitted {
                continue;
            }
            let id = match patch.pr_number {
                Some(id) => id,
                None => continue,
            };
            if let Some(phid) = get_latest_diff_phid(repo, id) {
                state.insert(format!("D{}", id), phid);
            }
        }
        repo.write_sync_state(&state);
    }
}

/// Parse a Phabricator revision ID from a commit message.
/// Looks for "Differential Revision: .../DXXXX" or just "D" followed by digits.
fn parse_revision_id(message: &str) -> Option<u32> {
    parse_revision_id_and_url(message).map(|(id, _)| id)
}

/// Find the revision ID of the closest submitted parent in the stack.
fn find_parent_revision(patches: &[PatchEntry], index: usize) -> Option<u32> {
    if index == 0 {
        return None;
    }
    for i in (0..index).rev() {
        if let Some(id) = patches[i].pr_number {
            if patches[i].status == PatchStatus::Submitted {
                return Some(id);
            }
        }
    }
    None
}

/// Find the commit hash in the current stack that has the given revision ID
/// in its Differential Revision trailer. Used after rebase when hashes change.
fn find_commit_with_revision(repo: &Repo, revision_id: Option<u32>) -> Result<String> {
    let target_id = revision_id.ok_or_else(|| eyre!("No revision ID to search for"))?;
    let base = repo.base()?;
    let log = repo.git_pub(&["log", "--format=%H %B", &format!("{}..HEAD", base)])?;

    // Each commit is: <hash> <full message body>
    // Commits are separated by the next line starting with a 40+ char hash
    let mut current_hash = String::new();
    let mut current_body = String::new();

    for line in log.lines() {
        // Check if this line starts a new commit (40-char hex hash)
        let is_new_commit = line.len() >= 40
            && line.chars().take(40).all(|c| c.is_ascii_hexdigit())
            && line.chars().nth(40).is_none_or(|c| c == ' ');

        if is_new_commit {
            // Check the previous commit
            if !current_hash.is_empty() {
                if let Some(id) = parse_revision_id(&current_body) {
                    if id == target_id {
                        return Ok(current_hash);
                    }
                }
            }
            current_hash = line.split_whitespace().next().unwrap_or("").to_string();
            current_body = line[current_hash.len()..].trim().to_string();
            current_body.push('\n');
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Check the last commit
    if !current_hash.is_empty() {
        if let Some(id) = parse_revision_id(&current_body) {
            if id == target_id {
                return Ok(current_hash);
            }
        }
    }

    Err(eyre!("Commit with D{} not found in stack", target_id))
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

/// Query Phabricator Conduit API for the latest diff PHID of a revision.
fn get_latest_diff_phid(repo: &Repo, revision_id: u32) -> Option<String> {
    let input = format!(r#"{{"constraints":{{"ids":[{}]}}}}"#, revision_id);

    // Use shell pipe to ensure stdin is properly closed
    let output = Command::new("sh")
        .current_dir(&repo.workdir)
        .args([
            "-c",
            &format!(
                "echo '{}' | arc call-conduit -- differential.revision.search",
                input
            ),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    let out = output.ok()?;
    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).ok()?;

    // Response: {"response":{"data":[{"fields":{"diffPHID":"PHID-DIFF-xxx"}}]}}
    json["response"]["data"][0]["fields"]["diffPHID"]
        .as_str()
        .map(|s| s.to_string())
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
        assert_eq!(
            parse_revision_id("Differential Revision: D5678"),
            Some(5678)
        );
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
