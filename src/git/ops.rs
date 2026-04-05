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
                pr_number: None,
                status: PatchStatus::Clean,
            });
        }
        Ok(patches)
    }

    /// Check if rebasing would cause conflicts for a commit.
    /// Returns list of conflicting file paths, or empty if clean.
    pub fn check_conflicts(&self, commit_hash: &str) -> Result<Vec<String>> {
        // Try a dry-run cherry-pick to detect conflicts
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .args(["cherry-pick", "--no-commit", commit_hash])
            .output()?;

        if result.status.success() {
            // Clean — reset the working tree
            let _ = self.git(&["reset", "--hard", "HEAD"]);
            return Ok(vec![]);
        }

        // Conflicts detected — gather the list
        let status_output = self.git(&["diff", "--name-only", "--diff-filter=U"])?;
        let conflicts: Vec<String> = status_output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        // Abort the cherry-pick
        let _ = self.git(&["cherry-pick", "--abort"]);

        Ok(conflicts)
    }

    /// Get short diff stat for a commit.
    pub fn diff_stat(&self, hash: &str) -> Result<String> {
        self.git(&["show", "--stat", "--format=", hash])
    }

    /// Get the full diff for a commit.
    pub fn diff_full(&self, hash: &str) -> Result<String> {
        self.git(&["show", "--format=", hash])
    }

    /// Check if there are any staged or unstaged changes.
    pub fn has_changes(&self) -> Result<bool> {
        let output = self.git(&["status", "--porcelain"])?;
        Ok(!output.trim().is_empty())
    }

    /// Stage all changes.
    pub fn add_all(&self) -> Result<()> {
        self.git(&["add", "-A"])?;
        Ok(())
    }

    /// Create a commit with the given message.
    pub fn commit(&self, message: &str) -> Result<()> {
        self.git(&["commit", "-m", message])?;
        Ok(())
    }

    /// Amend the current commit (keeps the message, adds staged changes).
    pub fn commit_amend_no_edit(&self) -> Result<()> {
        self.git(&["commit", "--amend", "--no-edit"])?;
        Ok(())
    }

    /// Start a rebase onto the base branch.
    /// Returns Ok(true) if clean, Ok(false) if conflicts need resolving.
    pub fn rebase_onto_base(&self) -> Result<bool> {
        let base = self.detect_base()?;
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

    /// Run a user-defined submit command for a specific commit.
    /// Temporarily checks out the target commit, runs the command, then
    /// checks out the original branch. The command template can contain
    /// `{hash}` and `{subject}` placeholders.
    pub fn run_submit_cmd(&self, cmd_template: &str, hash: &str, subject: &str) -> Result<String> {
        let cmd = cmd_template
            .replace("{hash}", hash)
            .replace("{subject}", subject);

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

