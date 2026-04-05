use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use crate::core::stack::{PatchEntry, PatchStatus};

/// Wrapper around a git repository.
pub struct Repo {
    pub workdir: PathBuf,
}

impl Repo {
    /// Open the repo containing the current directory.
    pub fn open() -> Result<Self> {
        let output = git_global(&["rev-parse", "--show-toplevel"])?;
        let workdir = PathBuf::from(output.trim());
        Ok(Self { workdir })
    }

    /// Detect the base branch (origin/main, origin/master, main, master).
    pub fn detect_base(&self) -> Result<String> {
        for candidate in &["origin/main", "origin/master", "main", "master"] {
            if self.git(&["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
                return Ok(candidate.to_string());
            }
        }
        Err(eyre!(
            "Could not detect base branch. Set it with `pgit config --base <branch>`."
        ))
    }

    /// Get the current HEAD commit hash (full).
    pub fn get_head_hash(&self) -> Result<String> {
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    /// Get the current branch name.
    pub fn get_current_branch(&self) -> Result<String> {
        Ok(self.git(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string())
    }

    /// Hard-reset the current branch to a specific commit.
    /// Used by undo/redo to restore git history.
    pub fn reset_hard(&self, hash: &str) -> Result<()> {
        self.git(&["reset", "--hard", hash])?;
        Ok(())
    }

    /// List commits between base and HEAD, bottom-of-stack first.
    ///
    /// Uses a record separator (%x1e) between commits and a unit separator
    /// (%x1f) between fields so that multiline commit bodies don't break parsing.
    /// After loading, checks which commits have submitted PRs.
    pub fn list_stack_commits(&self) -> Result<Vec<PatchEntry>> {
        let base = self.detect_base()?;
        let range = format!("{}..HEAD", base);
        let format = "%H%x1f%s%x1f%b%x1f%an%x1f%ai%x1e";
        let output = self.git(&["log", "--reverse", &format!("--format={}", format), &range])?;

        let mut patches = Vec::new();
        for record in output.split('\x1e') {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            let parts: Vec<&str> = record.splitn(5, '\x1f').collect();
            if parts.len() < 5 {
                continue;
            }
            patches.push(PatchEntry {
                hash: parts[0].to_string(),
                subject: parts[1].to_string(),
                body: parts[2].trim().to_string(),
                author: parts[3].to_string(),
                timestamp: parts[4].trim().to_string(),
                pr_branch: None,
                pr_number: None,
                status: PatchStatus::Clean,
            });
        }

        // Mark commits that have a corresponding pgit branch as Submitted
        self.mark_submitted_patches(&mut patches);

        Ok(patches)
    }

    /// Check each commit against open GitHub PRs and mark as Submitted.
    /// Only marks commits whose pgit branch has an OPEN PR on GitHub.
    /// Falls back to local branch check if `gh` CLI is unavailable.
    fn mark_submitted_patches(&self, patches: &mut [PatchEntry]) {
        // Fetch open PRs from GitHub — this is the source of truth
        let (pr_map, gh_available) = self.fetch_open_prs();

        for patch in patches.iter_mut() {
            let branch = self.make_pgit_branch_name(&patch.subject);

            if gh_available {
                // gh is available — only mark if there's an open PR
                if let Some(&pr_num) = pr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(pr_num);
                }
            } else {
                // gh unavailable — fall back to local branch existence
                if self.git(&["rev-parse", "--verify", &branch]).is_ok() {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                }
            }
        }
    }

    /// Query GitHub for OPEN PRs on pgit/* branches.
    /// Returns (branch→PR_number map, whether gh was available).
    fn fetch_open_prs(&self) -> (std::collections::HashMap<String, u32>, bool) {
        let mut map = std::collections::HashMap::new();
        let output = Command::new("gh")
            .current_dir(&self.workdir)
            .args(["pr", "list", "--state", "open", "--json", "number,headRefName", "--limit", "100"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let json = String::from_utf8_lossy(&out.stdout);
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                    for pr in prs {
                        if let (Some(num), Some(head)) = (
                            pr["number"].as_u64(),
                            pr["headRefName"].as_str(),
                        ) {
                            if head.starts_with("pgit/") {
                                map.insert(head.to_string(), num as u32);
                            }
                        }
                    }
                }
                (map, true)
            }
            _ => (map, false), // gh not available
        }
    }

    /// Get the full diff for a commit.
    pub fn diff_full(&self, hash: &str) -> Result<String> {
        self.git(&["show", "--format=", hash])
    }

    /// Fetch from origin to ensure we have the latest remote state.
    pub fn fetch_origin(&self) -> Result<()> {
        self.git(&["fetch", "origin"])?;
        Ok(())
    }

    /// Fetch from origin and rebase onto the base branch.
    /// Reports progress via callback.
    /// Returns Ok(true) if clean, Ok(false) if conflicts need resolving.
    pub fn rebase_onto_base(&self, on_progress: &dyn Fn(&str)) -> Result<bool> {
        let base = self.detect_base()?;

        on_progress("Fetching from origin...");
        let _ = self.fetch_origin();

        on_progress(&format!("Rebasing onto {}...", base));
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .args(["rebase", &base])
            .output()?;

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false);
        }
        Err(eyre!("Rebase failed: {}", stderr))
    }

    /// Continue a rebase after conflicts have been resolved and staged.
    /// Returns Ok(true) if rebase completed, Ok(false) if more conflicts.
    pub fn rebase_continue(&self) -> Result<bool> {
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_EDITOR", "true") // auto-accept commit messages
            .args(["rebase", "--continue"])
            .output()?;

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false);
        }
        Err(eyre!("Rebase continue failed: {}", stderr))
    }

    /// Abort an in-progress rebase.
    pub fn rebase_abort(&self) -> Result<()> {
        self.git(&["rebase", "--abort"])?;
        Ok(())
    }

    /// Start an interactive rebase with a specific commit marked as "edit".
    /// Git will replay commits up to that point and pause, letting the user
    /// modify the working tree. Returns Ok(false) if paused for editing,
    /// Ok(true) if the commit wasn't in range (shouldn't normally happen).
    pub fn rebase_edit_commit(&self, short_hash: &str) -> Result<bool> {
        let base = self.detect_base()?;
        let sed_cmd = format!(
            "sed -i 's/^pick {}/edit {}/'",
            short_hash, short_hash
        );
        let _result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        // git rebase -i with "edit" returns exit 0 even when paused.
        // The reliable check is whether the rebase-merge dir exists.
        if self.is_rebase_in_progress() {
            return Ok(false); // paused for editing
        }
        Ok(true) // completed without stopping
    }

    /// Start an interactive rebase with a "break" inserted after a specific
    /// commit. This pauses the rebase so the user can insert a new commit.
    pub fn rebase_break_after(&self, short_hash: &str) -> Result<bool> {
        let base = self.detect_base()?;
        let sed_cmd = format!(
            "sed -i '/^pick {}/a break'",
            short_hash
        );
        let _result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        if self.is_rebase_in_progress() {
            return Ok(false); // paused at break
        }
        Ok(true) // completed (break wasn't hit)
    }

    /// Squash multiple commits into one via interactive rebase, using a custom
    /// commit message. `hashes` should be short hashes ordered from oldest to
    /// newest. The first hash stays as `pick`, the rest become `squash`.
    /// The `message` is used as the final commit message for the squashed result.
    /// Returns Ok(true) if clean, Ok(false) if conflicts.
    pub fn squash_commits_with_message(&self, hashes: &[String], message: &str) -> Result<bool> {
        if hashes.len() < 2 {
            return Err(eyre!("Need at least 2 commits to squash"));
        }
        let base = self.detect_base()?;

        // Build sed: first hash stays pick, rest become squash
        let sed_parts: Vec<String> = hashes[1..]
            .iter()
            .map(|h| format!("s/^pick {}/squash {}/", h, h))
            .collect();
        let seq_editor = format!("sed -i '{}'", sed_parts.join("; "));

        // Write desired message to temp file. GIT_EDITOR will copy it over
        // git's proposed squash message when prompted.
        let msg_file = std::env::temp_dir().join(format!(
            "pgit-squash-msg-{}.txt",
            std::process::id()
        ));
        std::fs::write(&msg_file, message)?;
        let msg_editor = format!("cp {} ", msg_file.display());

        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &seq_editor)
            .env("GIT_EDITOR", &msg_editor)
            .args(["rebase", "-i", &base])
            .output()?;

        let _ = std::fs::remove_file(&msg_file);

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false);
        }
        Err(eyre!("Squash failed: {}", stderr))
    }

    /// Remove a commit from git history via interactive rebase.
    /// Returns Ok(true) if clean, Ok(false) if conflicts.
    pub fn remove_commit(&self, short_hash: &str) -> Result<bool> {
        let base = self.detect_base()?;
        // Change "pick <hash>" to "drop <hash>" in the rebase todo
        let sed_cmd = format!(
            "sed -i 's/^pick {}/drop {}/'",
            short_hash, short_hash
        );
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true); // removed cleanly
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false); // conflicts
        }
        Err(eyre!("Remove commit failed: {}", stderr))
    }

    /// Swap two adjacent commits in git history via interactive rebase.
    /// `hash_a` and `hash_b` should be short hashes of adjacent commits
    /// where `hash_a` is currently below (older) and `hash_b` is above (newer).
    /// After swapping, `hash_a` will be above `hash_b`.
    /// Returns Ok(true) if clean, Ok(false) if conflicts.
    pub fn swap_commits(&self, hash_below: &str, hash_above: &str) -> Result<bool> {
        let base = self.detect_base()?;

        // Strategy: in the rebase todo, the older commit (hash_below) appears
        // first. We want to swap their order. Use sed to:
        // 1. When we see the line for hash_below, hold it and delete
        // 2. When we see the line for hash_above, print it, then print the held line
        let sed_cmd = format!(
            "sed -i '/^pick {}/{{ h; d }}; /^pick {}/{{ p; x }}'",
            hash_below, hash_above
        );
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true); // swapped cleanly
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false); // conflicts
        }
        Err(eyre!("Swap commits failed: {}", stderr))
    }

    /// Check if a rebase is currently in progress.
    pub fn is_rebase_in_progress(&self) -> bool {
        self.workdir.join(".git/rebase-merge").exists()
            || self.workdir.join(".git/rebase-apply").exists()
    }

    /// Get the list of files with conflicts (unmerged paths).
    pub fn conflicted_files(&self) -> Result<Vec<String>> {
        let output = self.git(&["diff", "--name-only", "--diff-filter=U"])?;
        Ok(output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// Submit a single commit as a GitHub PR using the `gh` CLI.
    /// Creates a branch `pgit/<subject>` for the commit and creates a PR
    /// with the specified base branch.
    pub fn github_submit(
        &self,
        commit_hash: &str,
        subject: &str,
        pr_base: &str,
        body: &str,
    ) -> Result<String> {
        let branch = self.get_current_branch()?;
        let branch_name = self.make_pgit_branch_name(subject);

        // Create and push the commit's branch
        self.git(&["branch", "-f", &branch_name, commit_hash])?;
        self.git(&["push", "-f", "origin", &branch_name])?;

        // Try to create PR via gh
        let create = Command::new("gh")
            .current_dir(&self.workdir)
            .args([
                "pr", "create",
                "--head", &branch_name,
                "--base", pr_base,
                "--title", subject,
                "--body", body,
            ])
            .output()?;

        // Checkout back to original branch
        let _ = self.git(&["checkout", "--quiet", &branch]);

        if create.status.success() {
            let url = String::from_utf8_lossy(&create.stdout).trim().to_string();
            return Ok(format!("PR created: {} → {}", url, pr_base));
        }

        let stderr = String::from_utf8_lossy(&create.stderr);
        if stderr.contains("already exists") {
            // PR exists — update its base branch
            self.edit_pr_base(&branch_name, pr_base);
            return Ok(format!("PR updated: {} → {}", branch_name, pr_base));
        }

        Err(eyre!("gh pr create failed: {}", stderr))
    }


    /// Force-push a pgit branch and update the PR base.
    pub fn update_pr(
        &self,
        commit_hash: &str,
        branch_name: &str,
        pr_base: &str,
    ) -> Result<String> {
        // Fetch to ensure we have latest state
        let _ = self.fetch_origin();

        // Update the branch to point at the current commit
        self.git(&["branch", "-f", branch_name, commit_hash])?;
        self.git(&["push", "-f", "origin", branch_name])?;

        // Update PR base via gh
        let base_updated = self.edit_pr_base(branch_name, pr_base);

        if base_updated {
            Ok(format!("PR updated: {} → {}", branch_name, pr_base))
        } else {
            Ok(format!("PR pushed: {} (base update to {} may need manual action)", branch_name, pr_base))
        }
    }

    /// Determine the correct PR base for a commit by walking down the stack.
    /// Checks which parent PRs are still open. If all parents below are
    /// merged/closed, returns main.
    pub fn determine_base_for_commit(
        &self,
        patches: &[crate::core::stack::PatchEntry],
        commit_index: usize,
    ) -> String {
        let base = self.detect_base().unwrap_or_else(|_| "main".into());
        let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();

        if commit_index == 0 {
            return base_branch;
        }

        let (open_prs, gh_available) = self.fetch_open_prs();

        // Walk down from the parent below cursor to the bottom of the stack
        for j in (0..commit_index).rev() {
            let parent = &patches[j];
            let parent_branch = self.make_pgit_branch_name(&parent.subject);

            if gh_available {
                if open_prs.contains_key(&parent_branch) {
                    // Parent still has an open PR — use its branch
                    let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                    let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                    return parent_branch;
                }
                // Parent's PR merged/closed — skip, keep looking
            } else {
                // gh not available — fall back to local branch check
                if self.git(&["rev-parse", "--verify", &parent_branch]).is_ok() {
                    let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                    let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                    return parent_branch;
                }
            }
        }

        // All parents merged or not submitted — base is main
        base_branch
    }

    /// Generate a stable branch name like `pgit/hokwang/feat-add-login`.
    /// Includes the git username to avoid conflicts with other pgit users.
    /// Does NOT include the hash so the name stays the same when the commit
    /// is edited/amended — allowing `git push -f` to update an existing PR.
    pub fn make_pgit_branch_name(&self, subject: &str) -> String {
        let user = self.get_pgit_username();
        let sanitized: String = subject
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let sanitized = sanitized.trim_matches('-');
        let truncated = &sanitized[..50.min(sanitized.len())];
        format!("pgit/{}/{}", user, truncated.trim_end_matches('-'))
    }

    /// Get a short, sanitized username for branch naming.
    /// Uses git config user.name, falls back to system user.
    fn get_pgit_username(&self) -> String {
        let name = self.git(&["config", "user.name"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let name = if name.is_empty() {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "user".to_string())
        } else {
            name
        };

        // Sanitize: lowercase, alphanumeric + dash, max 20 chars
        let sanitized: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let sanitized = sanitized.trim_matches('-');
        sanitized[..20.min(sanitized.len())].trim_end_matches('-').to_string()
    }

    /// Find local pgit/* branches whose PRs are merged or closed.
    pub fn find_stale_branches(&self) -> Vec<String> {
        let user = self.get_pgit_username();
        let prefix = format!("pgit/{}/", user);

        // List all local pgit branches for this user
        let local = self.git(&["branch", "--list", &format!("{}*", prefix), "--format=%(refname:short)"])
            .unwrap_or_default();
        let local_branches: Vec<String> = local.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        if local_branches.is_empty() {
            return Vec::new();
        }

        // Get open PRs
        let (open_prs, gh_available) = self.fetch_open_prs();
        if !gh_available {
            return Vec::new();
        }

        // Stale = local branch exists but no open PR
        local_branches.into_iter()
            .filter(|b| !open_prs.contains_key(b))
            .collect()
    }

    /// Delete branches both locally and on the remote.
    pub fn delete_branches(&self, branches: &[String]) {
        for branch in branches {
            let _ = self.git(&["branch", "-D", branch]);
            let _ = self.git(&["push", "origin", "--delete", branch]);
        }
    }

    /// Update a PR's base branch. Uses the PR number when available for
    /// reliability, and falls back to branch-based lookup.
    /// Suppresses stderr (gh may emit GraphQL deprecation warnings).
    fn edit_pr_base(&self, branch: &str, base: &str) -> bool {
        // Try to get the PR number — more reliable than branch-based edit
        let number = self.get_pr_number(branch);

        let target = match &number {
            Some(n) => n.as_str(),
            None => branch,
        };

        // Use gh pr edit with the PR number (or branch as fallback)
        let result = Command::new("gh")
            .current_dir(&self.workdir)
            .args(["pr", "edit", target, "--base", base])
            .stderr(std::process::Stdio::null())
            .output();

        if let Ok(ref out) = result {
            if out.status.success() {
                return true;
            }
        }

        // If that failed, try via REST API as last resort
        if let Some(n) = &number {
            let api_path = format!("repos/{{owner}}/{{repo}}/pulls/{}", n);
            let base_field = format!("base={}", base);
            let api_result = Command::new("gh")
                .current_dir(&self.workdir)
                .args(["api", "-X", "PATCH", &api_path, "-f", &base_field])
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .output();
            return api_result.map(|o| o.status.success()).unwrap_or(false);
        }

        false
    }

    /// Look up the PR number for a head branch.
    fn get_pr_number(&self, branch: &str) -> Option<String> {
        let output = Command::new("gh")
            .current_dir(&self.workdir)
            .args(["pr", "view", branch, "--json", "number", "-q", ".number"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            let num = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !num.is_empty() { Some(num) } else { None }
        } else {
            None
        }
    }

    /// Sync all submitted PRs: fetch latest state, force-push branches,
    /// and update PR bases. For any parent whose PR is no longer open
    /// (merged or closed), updates the child PR base to main.
    /// Calls `on_progress` with status messages for each step.
    pub fn sync_pr_bases(
        &self,
        patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Fetching latest from origin...");
        let _ = self.fetch_origin();

        let base = self.detect_base()?;
        let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();

        on_progress("Checking open PRs on GitHub...");
        let (open_prs, _) = self.fetch_open_prs();
        let mut updates = Vec::new();

        for (i, patch) in patches.iter().enumerate() {
            let branch = self.make_pgit_branch_name(&patch.subject);

            // Only sync commits that have an open PR
            if !open_prs.contains_key(&branch) {
                continue;
            }

            on_progress(&format!("Syncing: {} ...", &patch.subject));

            // Determine what the PR base should be.
            // Walk down the stack: if a parent's PR is still open, use its
            // branch as base. If the parent's PR is closed/merged (not in
            // open_prs), skip it — the base should be main.
            let correct_base = if i == 0 {
                base_branch.clone()
            } else {
                let mut base_for_pr = base_branch.clone();
                for j in (0..i).rev() {
                    let parent = &patches[j];
                    let parent_branch = self.make_pgit_branch_name(&parent.subject);
                    if open_prs.contains_key(&parent_branch) {
                        // Parent still has an open PR — use its branch
                        let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                        let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                        base_for_pr = parent_branch;
                        break;
                    }
                    // Parent's PR is merged/closed — skip, keep looking
                }
                base_for_pr
            };

            // Force-push this commit's branch
            let _ = self.git(&["branch", "-f", &branch, &patch.hash]);
            let _ = self.git(&["push", "-f", "origin", &branch]);

            // Update PR base via gh
            let edited = self.edit_pr_base(&branch, &correct_base);
            let status = if edited { "✓" } else { "⚠" };
            updates.push(format!("{} {} → {}", status, branch, correct_base));
        }

        Ok(updates)
    }

    /// Run a user-defined submit command for a specific commit.
    /// Temporarily checks out the target commit, runs the command, then
    /// checks out the original branch. The command template can contain
    /// `{hash}`, `{subject}`, `{message}`, and `{message_file}` placeholders.
    pub fn run_submit_cmd(&self, cmd_template: &str, hash: &str, subject: &str, body: &str) -> Result<String> {
        // Write message to a temp file for {message_file} placeholder
        let msg_file = std::env::temp_dir().join(format!("pgit-submit-msg-{}.txt", std::process::id()));
        std::fs::write(&msg_file, body)?;

        let cmd = cmd_template
            .replace("{hash}", hash)
            .replace("{subject}", subject)
            .replace("{message}", body)
            .replace("{message_file}", &msg_file.display().to_string());

        // Save current branch so we can return after the command
        let branch = self.get_current_branch()?;

        // Checkout the target commit (detached HEAD)
        self.git(&["checkout", "--quiet", hash])?;

        let result = Command::new("sh")
            .current_dir(&self.workdir)
            .args(["-c", &cmd])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();

        // Always checkout back, even if the command failed
        let _ = self.git(&["checkout", "--quiet", &branch]);
        let _ = std::fs::remove_file(&msg_file);

        if !result.status.success() {
            return Err(eyre!("Submit command failed: {}{}", stdout, stderr));
        }
        Ok(format!("{}{}", stdout, stderr))
    }

    /// Run a git command inside this repo's workdir.
    fn git(&self, args: &[&str]) -> Result<String> {
        git_in(&self.workdir, args)
    }
}

/// Run a git command in a specific directory and return stdout.
fn git_in(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!("git {} failed: {}", args.join(" "), stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a git command without a specific workdir (uses cwd).
fn git_global(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!("git {} failed: {}", args.join(" "), stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

