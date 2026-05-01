use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use crate::core::stack::{PatchEntry, PatchStatus};
use crate::forge::ForgeKind;

/// `sed` plus `-i` flags for in-place edits passed to `GIT_SEQUENCE_EDITOR` (run under `sh -c`).
/// BSD/macOS `sed` requires a backup extension after `-i` (`''` means none); GNU `sed` accepts
/// the same form, so this keeps rebase todo editing working on both.
pub(crate) fn sed_inplace_shell_prefix() -> &'static str {
    match std::env::consts::OS {
        "macos" | "freebsd" | "openbsd" | "netbsd" | "dragonfly" => "sed -i ''",
        _ => "sed -i",
    }
}

/// Wrapper around a git repository.
pub struct Repo {
    pub workdir: PathBuf,
    /// When set (via [`Self::with_resolved_base`]), used for stack operations instead of auto-detect.
    resolved_base: Option<String>,
}

impl Repo {
    /// Open the repo containing the current directory.
    pub fn open() -> Result<Self> {
        let output = git_global(&["rev-parse", "--show-toplevel"])?;
        let workdir = PathBuf::from(output.trim());
        Ok(Self {
            workdir,
            resolved_base: None,
        })
    }

    /// Open a repo at `workdir` without resolving from cwd (for tests and tooling).
    pub fn at_dir(workdir: PathBuf) -> Self {
        Self {
            workdir,
            resolved_base: None,
        }
    }

    /// Attach a resolved base branch (e.g. from `.pilegit.toml`) for all stack git operations.
    pub fn with_resolved_base(self, base: String) -> Self {
        Self {
            workdir: self.workdir,
            resolved_base: Some(base),
        }
    }

    /// Resolve stack base: explicit `[repo].base`, then [`crate::forge::stack_base_hint`] for the
    /// configured forge (CLI default branch when implemented), then [`Self::detect_base`].
    pub fn resolve_base(&self, configured: Option<&str>, forge: ForgeKind) -> Result<String> {
        if let Some(b) = configured {
            let b = b.trim();
            if !b.is_empty() {
                if self.git(&["rev-parse", "--verify", "--quiet", b]).is_ok() {
                    return Ok(b.to_string());
                }
                return Err(eyre!(
                    "Configured base branch {:?} is not a valid ref. \
                     Fix repo.base in .pilegit.toml or fetch that branch from your remote.",
                    b
                ));
            }
        }
        if let Some(b) = crate::forge::stack_base_hint::try_from_forge_cli(self, forge) {
            return Ok(b);
        }
        self.detect_base()
    }

    /// Base ref for the stack: [`Self::resolved_base`] if set, else auto-detect.
    pub fn base(&self) -> Result<String> {
        if let Some(ref b) = self.resolved_base {
            return Ok(b.clone());
        }
        self.detect_base()
    }

    /// Detect the base branch (origin/main, origin/master, main, master).
    pub fn detect_base(&self) -> Result<String> {
        for candidate in &["origin/main", "origin/master", "main", "master"] {
            if self
                .git(&["rev-parse", "--verify", "--quiet", candidate])
                .is_ok()
            {
                return Ok(candidate.to_string());
            }
        }
        Err(eyre!(
            "Could not detect base branch (tried forge CLI default for your `[forge].type`, \
             then origin/main, origin/master, main, master). \
             Set repo.base in .pilegit.toml (for example base = \"origin/develop\") or run `pgit init`."
        ))
    }

    /// Get the current HEAD commit hash (full).
    pub fn get_head_hash(&self) -> Result<String> {
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    /// Get the current branch name.
    pub fn get_current_branch(&self) -> Result<String> {
        Ok(self
            .git(&["rev-parse", "--abbrev-ref", "HEAD"])?
            .trim()
            .to_string())
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
        let base = self.base()?;
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
                pr_url: None,
                status: PatchStatus::Clean,
            });
        }

        Ok(patches)
    }

    /// Get the full diff for a commit.
    pub fn diff_full(&self, hash: &str) -> Result<String> {
        self.git(&["show", "--format=", hash])
    }

    /// Check if there are uncommitted changes (staged or unstaged).
    /// Ignores .pilegit.toml since pgit creates it.
    pub fn has_uncommitted_changes(&self) -> bool {
        let output = Command::new("git")
            .current_dir(&self.workdir)
            .args(["status", "--porcelain"])
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.lines().any(|l| !l.ends_with(".pilegit.toml"))
            }
            Err(_) => false,
        }
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
        let base = self.base()?;

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
        if stderr.contains("CONFLICT")
            || stderr.contains("could not apply")
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
        if stderr.contains("CONFLICT")
            || stderr.contains("could not apply")
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

    /// Get git's own abbreviated hash for a commit.
    /// This ensures sed patterns match the rebase todo format.
    pub fn abbrev(&self, hash: &str) -> String {
        self.git(&["rev-parse", "--short", hash])
            .unwrap_or_else(|_| hash.to_string())
            .trim()
            .to_string()
    }

    /// Start an interactive rebase with a specific commit marked as "edit".
    /// Git will replay commits up to that point and pause, letting the user
    /// modify the working tree. Returns Ok(false) if paused for editing,
    /// Ok(true) if the commit wasn't in range (shouldn't normally happen).
    pub fn rebase_edit_commit(&self, short_hash: &str) -> Result<bool> {
        let base = self.base()?;
        let abbr = self.abbrev(short_hash);
        let sed_cmd = format!(
            "{} 's/^pick {}/edit {}/'",
            sed_inplace_shell_prefix(),
            abbr,
            abbr
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
        let base = self.base()?;
        let abbr = self.abbrev(short_hash);
        let sed_cmd = format!("{} '/^pick {}/a break'", sed_inplace_shell_prefix(), abbr);
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
        let base = self.base()?;

        // Build sed: first hash stays pick, rest become squash
        let sed_parts: Vec<String> = hashes[1..]
            .iter()
            .map(|h| {
                let abbr = self.abbrev(h);
                format!("s/^pick {}/squash {}/", abbr, abbr)
            })
            .collect();
        let seq_editor = format!("{} '{}'", sed_inplace_shell_prefix(), sed_parts.join("; "));

        // Write desired message to temp file. GIT_EDITOR will copy it over
        // git's proposed squash message when prompted.
        let msg_file =
            std::env::temp_dir().join(format!("pgit-squash-msg-{}.txt", std::process::id()));
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
        if stderr.contains("CONFLICT")
            || stderr.contains("could not apply")
            || self.is_rebase_in_progress()
        {
            return Ok(false);
        }
        Err(eyre!("Squash failed: {}", stderr))
    }

    /// Remove a commit from git history via interactive rebase.
    /// Returns Ok(true) if clean, Ok(false) if conflicts.
    pub fn remove_commit(&self, short_hash: &str) -> Result<bool> {
        let base = self.base()?;
        let abbr = self.abbrev(short_hash);
        // Change "pick <hash>" to "drop <hash>" in the rebase todo
        let sed_cmd = format!(
            "{} 's/^pick {}/drop {}/'",
            sed_inplace_shell_prefix(),
            abbr,
            abbr
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
        if stderr.contains("CONFLICT")
            || stderr.contains("could not apply")
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
        let base = self.base()?;

        let abbrev_below = self.abbrev(hash_below);
        let abbrev_above = self.abbrev(hash_above);

        // Rebase todo lists older commits first. We need to swap two adjacent
        // `pick` lines. An earlier implementation used GNU `sed` hold-space
        // (`h`, `d`, `p`, `x` inside `{ ... }`) as GIT_SEQUENCE_EDITOR; that
        // breaks on macOS for two reasons: (1) BSD `sed -i` requires a backup
        // suffix (`-i ''`), and even with that fixed, (2) BSD `sed` does not
        // accept the same one-line `{ cmd; cmd }` syntax as GNU sed, so the
        // editor failed with errors like "extra characters at the end of d".
        // Perl performs one multiline substitution with identical behavior on
        // typical Linux and macOS developer machines (`perl` is in the base OS).
        let seq_editor = format!(
            r#"perl -0777 -i -pe 's/(^pick {}[^\n]*\n)(^pick {}[^\n]*)/$2\n$1/m'"#,
            abbrev_below, abbrev_above
        );
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &seq_editor)
            .args(["rebase", "-i", &base])
            .output()?;

        if result.status.success() && !self.is_rebase_in_progress() {
            return Ok(true); // swapped cleanly
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT")
            || stderr.contains("could not apply")
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

    /// Determine the correct PR base for a commit by walking down the stack.
    /// Checks which parent PRs are still open. If all parents below are
    /// merged/closed, returns main.
    /// Determine the correct PR base for a commit by walking down the stack.
    /// Accepts the open PR map from the forge's list_open() method.
    pub fn determine_base_for_commit(
        &self,
        patches: &[crate::core::stack::PatchEntry],
        commit_index: usize,
        open_prs: &std::collections::HashMap<String, u32>,
        gh_available: bool,
    ) -> String {
        let base = self.base().unwrap_or_else(|_| "main".into());
        let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();

        if commit_index == 0 {
            return base_branch;
        }

        for j in (0..commit_index).rev() {
            let parent = &patches[j];
            let parent_branch = self.make_pgit_branch_name(&parent.subject);

            if gh_available {
                if open_prs.contains_key(&parent_branch) {
                    let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                    let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                    return parent_branch;
                }
            } else if self.git(&["rev-parse", "--verify", &parent_branch]).is_ok() {
                let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                return parent_branch;
            }
        }

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
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let sanitized = sanitized.trim_matches('-');
        let truncated = &sanitized[..50.min(sanitized.len())];
        format!("pgit/{}/{}", user, truncated.trim_end_matches('-'))
    }

    /// Get a short, sanitized username for branch naming.
    /// Uses git config user.name, falls back to system user.
    fn get_pgit_username(&self) -> String {
        let name = self
            .git(&["config", "user.name"])
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
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let sanitized = sanitized.trim_matches('-');
        sanitized[..20.min(sanitized.len())]
            .trim_end_matches('-')
            .to_string()
    }

    /// List all local pgit branches for the current user.
    pub fn list_pgit_branches(&self) -> Vec<String> {
        let user = self.get_pgit_username();
        let prefix = format!("pgit/{}/", user);
        let local = self
            .git(&[
                "branch",
                "--list",
                &format!("{}*", prefix),
                "--format=%(refname:short)",
            ])
            .unwrap_or_default();
        local
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Check if a branch's tip commit is an ancestor of the base branch.
    /// True for merge-commit and fast-forward merges where the original
    /// commit is preserved.
    pub fn branch_is_in_base(&self, branch: &str) -> bool {
        let base = self.base().unwrap_or_else(|_| "origin/main".to_string());
        self.git(&["merge-base", "--is-ancestor", branch, &base])
            .is_ok()
    }

    /// Find stale pgit branches using the forge's open PR list.
    /// Returns branches that are either:
    ///   - Not in the open PR list (closed/merged on the forge), or
    ///   - Whose commit is now reachable from the base branch
    ///
    /// For forge-specific stale detection (e.g. Phabricator trailer matching),
    /// the forge implementation should provide additional logic.
    pub fn find_stale_branches_with(
        &self,
        open_prs: &std::collections::HashMap<String, u32>,
        gh_available: bool,
    ) -> Vec<String> {
        let local_branches = self.list_pgit_branches();
        if local_branches.is_empty() {
            return Vec::new();
        }

        local_branches
            .into_iter()
            .filter(|b| {
                // Only trust the open_prs check if the listing returned at least
                // one result. An empty listing likely means the CLI query failed
                // or returned no matches — treating all branches as stale would
                // be destructive.
                if gh_available && !open_prs.is_empty() && !open_prs.contains_key(b) {
                    return true;
                }
                // Stale if branch's commit is now an ancestor of base
                self.branch_is_in_base(b)
            })
            .collect()
    }

    /// Delete branches locally and on the remote.
    /// Only called after confirming branches are stale (MR merged/closed).
    pub fn delete_branches(&self, branches: &[String]) {
        for branch in branches {
            let _ = self.git(&["branch", "-D", branch]);
            let _ = self.git(&["push", "origin", "--delete", branch]);
        }
    }

    /// Force-update a branch to point at a hash, then push.
    /// If we're already on the branch, skip the branch -f (it's already at HEAD).
    pub fn force_update_and_push(&self, branch: &str, hash: &str) -> Result<()> {
        let current = self.get_current_branch().unwrap_or_default();
        if current != branch {
            self.git(&["branch", "-f", branch, hash])?;
        }
        self.git(&["push", "-f", "origin", branch])?;
        Ok(())
    }

    /// Public git command for use by forge implementations.
    pub fn git_pub(&self, args: &[&str]) -> Result<String> {
        git_in(&self.workdir, args)
    }

    /// Read sync state from .git/pgit-sync-state.json.
    pub fn read_sync_state(&self) -> std::collections::HashMap<String, String> {
        let path = self.workdir.join(".git/pgit-sync-state.json");
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Write sync state to .git/pgit-sync-state.json.
    pub fn write_sync_state(&self, state: &std::collections::HashMap<String, String>) {
        let path = self.workdir.join(".git/pgit-sync-state.json");
        if let Ok(json) = serde_json::to_string_pretty(state) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Walk down the stack to determine the correct PR base for a commit.
    /// Uses the open_prs map to detect merged parents.
    pub fn walk_stack_for_base(
        &self,
        patches: &[crate::core::stack::PatchEntry],
        commit_index: usize,
        open_prs: &std::collections::HashMap<String, u32>,
        base_branch: &str,
    ) -> String {
        if commit_index == 0 {
            return base_branch.to_string();
        }

        for j in (0..commit_index).rev() {
            let parent = &patches[j];
            let parent_branch = self.make_pgit_branch_name(&parent.subject);
            if open_prs.contains_key(&parent_branch) {
                let _ = self.git(&["branch", "-f", &parent_branch, &parent.hash]);
                let _ = self.git(&["push", "-f", "origin", &parent_branch]);
                return parent_branch;
            }
        }
        base_branch.to_string()
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
