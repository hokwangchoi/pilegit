use std::collections::HashMap;
use std::process::Command;

use color_eyre::Result;

use super::Forge;
use crate::core::stack::PatchEntry;
use crate::git::ops::Repo;

/// Custom command forge — runs a user-defined command template.
pub struct Custom {
    cmd_template: String,
}

impl Custom {
    pub fn new(cmd_template: String) -> Self {
        Self { cmd_template }
    }
}

impl Forge for Custom {
    fn name(&self) -> &str { "Custom" }
    fn uses_branches(&self) -> bool { false }

    fn submit(
        &self, repo: &Repo, hash: &str, subject: &str,
        _base: &str, body: &str,
    ) -> Result<String> {
        // Write message to temp file for {message_file} placeholder
        let msg_file = std::env::temp_dir()
            .join(format!("pgit-submit-msg-{}.txt", std::process::id()));
        std::fs::write(&msg_file, body)?;

        let cmd = self.cmd_template
            .replace("{hash}", hash)
            .replace("{subject}", subject)
            .replace("{message}", body)
            .replace("{message_file}", &msg_file.display().to_string());

        let branch = repo.get_current_branch()?;
        repo.git_pub(&["checkout", "--quiet", hash])?;

        let result = Command::new("sh")
            .current_dir(&repo.workdir)
            .args(["-c", &cmd])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);
        let _ = std::fs::remove_file(&msg_file);

        if result.status.success() {
            Ok(format!("{}{}", stdout.trim(), stderr.trim()))
        } else {
            Ok(format!("Command exited with error: {}{}", stdout.trim(), stderr.trim()))
        }
    }

    fn update(
        &self, repo: &Repo, hash: &str, subject: &str, base: &str,
    ) -> Result<String> {
        self.submit(repo, hash, subject, base, "")
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool { true }

    fn mark_submitted(&self, _repo: &Repo, _patches: &mut [PatchEntry]) {}

    fn sync(
        &self, _repo: &Repo, _patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Custom commands don't support sync — re-submit individually with 'p'.");
        Ok(Vec::new())
    }
}
