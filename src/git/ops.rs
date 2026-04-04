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

    /// Start a rebase onto the base branch.
    /// Returns Ok(true) if clean, Ok(false) if conflicts need resolving.
    pub fn rebase_onto_base(&self) -> Result<bool> {
        let base = self.detect_base()?;
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .args(["rebase", &base])
            .output()?;

        if result.status.success() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        // "CONFLICT" in stderr means merge conflicts
        if stderr.contains("CONFLICT") || stderr.contains("could not apply") {
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

        if result.status.success() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("could not apply") {
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
    /// This pauses the rebase at that commit so the user can amend it.
    /// Returns Ok(true) if no rebase needed, Ok(false) if paused for editing.
    pub fn rebase_edit_commit(&self, short_hash: &str) -> Result<bool> {
        let base = self.detect_base()?;
        // sed command to change "pick <hash>" to "edit <hash>"
        let sed_cmd = format!(
            "sed -i 's/^pick {} /edit {} /'",
            short_hash, short_hash
        );
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        if result.status.success() {
            // Rebase completed without stopping — commit wasn't in range
            return Ok(true);
        }

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // "Stopped at" means rebase paused for editing
        if combined.contains("Stopped at") || combined.contains("edit") {
            return Ok(false);
        }

        Err(eyre!("Rebase edit failed: {}", combined))
    }

    /// Start an interactive rebase with a "break" inserted after a specific commit.
    /// This pauses the rebase so the user can insert a new commit at that point.
    /// Returns Ok(true) if completed (shouldn't happen), Ok(false) if paused.
    pub fn rebase_break_after(&self, short_hash: &str) -> Result<bool> {
        let base = self.detect_base()?;
        // sed command: after the line matching "pick <hash>", insert "break"
        let sed_cmd = format!(
            "sed -i '/^pick {} /a break'",
            short_hash
        );
        let result = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_SEQUENCE_EDITOR", &sed_cmd)
            .args(["rebase", "-i", &base])
            .output()?;

        let stderr = String::from_utf8_lossy(&result.stderr);
        // Exit code 0 with no output = completed (no break hit)
        // Usually the break causes a non-zero exit or a specific message
        if result.status.success() && !stderr.contains("break") {
            return Ok(true);
        }

        // Rebase paused at break point
        Ok(false)
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

    /// Run a user-defined submit command for the commit at HEAD~n.
    /// The command string can contain `{hash}` and `{subject}` placeholders.
    pub fn run_submit_cmd(&self, cmd_template: &str, hash: &str, subject: &str) -> Result<String> {
        let cmd = cmd_template
            .replace("{hash}", hash)
            .replace("{subject}", subject);

        let result = Command::new("sh")
            .current_dir(&self.workdir)
            .args(["-c", &cmd])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Helper to create a temporary git repo with commits.
    struct TestRepo {
        dir: PathBuf,
    }

    impl TestRepo {
        fn new() -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "pilegit-test-{}-{}",
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();

            let run = |args: &[&str]| {
                Command::new("git")
                    .current_dir(&dir)
                    .args(args)
                    .output()
                    .unwrap();
            };

            run(&["init"]);
            run(&["config", "user.email", "test@test.com"]);
            run(&["config", "user.name", "Test"]);

            // Initial commit, ensure branch is named "main"
            fs::write(dir.join("README.md"), "# test\n").unwrap();
            run(&["add", "."]);
            run(&["commit", "-m", "initial"]);
            run(&["branch", "-M", "main"]);

            Self { dir }
        }

        /// Create a fake origin/main ref pointing at the current HEAD.
        /// Call this before adding stack commits.
        fn set_origin_main(&self) {
            Command::new("git")
                .current_dir(&self.dir)
                .args(["update-ref", "refs/remotes/origin/main", "HEAD"])
                .output()
                .unwrap();
        }

        fn add_commit(&self, filename: &str, content: &str, message: &str) {
            fs::write(self.dir.join(filename), content).unwrap();
            Command::new("git")
                .current_dir(&self.dir)
                .args(["add", "."])
                .output()
                .unwrap();
            Command::new("git")
                .current_dir(&self.dir)
                .args(["commit", "-m", message])
                .output()
                .unwrap();
        }

        fn repo(&self) -> Repo {
            Repo {
                workdir: self.dir.clone(),
            }
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn test_detect_base_finds_origin_main() {
        let tr = TestRepo::new();
        tr.set_origin_main();
        let base = tr.repo().detect_base().unwrap();
        assert_eq!(base, "origin/main");
    }

    #[test]
    fn test_detect_base_falls_back_to_local_main() {
        let tr = TestRepo::new();
        // No origin/main — should fall back to local "main"
        let base = tr.repo().detect_base().unwrap();
        assert_eq!(base, "main");
    }

    #[test]
    fn test_list_stack_commits_empty_when_at_base() {
        let tr = TestRepo::new();
        // HEAD == main, so main..HEAD is empty
        let commits = tr.repo().list_stack_commits().unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn test_list_stack_commits_returns_ahead() {
        let tr = TestRepo::new();
        tr.set_origin_main();

        tr.add_commit("a.txt", "a", "feat: add a");
        tr.add_commit("b.txt", "b", "feat: add b");

        let commits = tr.repo().list_stack_commits().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "feat: add a");
        assert_eq!(commits[1].subject, "feat: add b");
        assert_eq!(commits[0].author, "Test");
        assert!(!commits[0].hash.is_empty());
    }

    #[test]
    fn test_list_stack_commits_multiline_body() {
        let tr = TestRepo::new();
        tr.set_origin_main();

        fs::write(tr.dir.join("c.txt"), "c").unwrap();
        Command::new("git")
            .current_dir(&tr.dir)
            .args(["add", "."])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(&tr.dir)
            .args(["commit", "-m", "subject\n\nline1\nline2\nline3"])
            .output()
            .unwrap();

        let commits = tr.repo().list_stack_commits().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "subject");
        assert!(commits[0].body.contains("line1"));
        assert!(commits[0].body.contains("line3"));
    }

    #[test]
    fn test_diff_stat() {
        let tr = TestRepo::new();
        tr.set_origin_main();
        tr.add_commit("file.txt", "hello\n", "add file");

        let commits = tr.repo().list_stack_commits().unwrap();
        let stat = tr.repo().diff_stat(&commits[0].hash).unwrap();
        assert!(stat.contains("file.txt"));
    }

    #[test]
    fn test_diff_full() {
        let tr = TestRepo::new();
        tr.set_origin_main();
        tr.add_commit("file.txt", "hello\n", "add file");

        let commits = tr.repo().list_stack_commits().unwrap();
        let diff = tr.repo().diff_full(&commits[0].hash).unwrap();
        assert!(diff.contains("+hello"));
    }
}
