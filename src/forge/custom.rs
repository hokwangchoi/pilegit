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
    fn name(&self) -> &str {
        "Custom"
    }
    fn needs_description_editor(&self) -> bool {
        false
    }

    fn submit(
        &self,
        repo: &Repo,
        hash: &str,
        subject: &str,
        _base: &str,
        body: &str,
    ) -> Result<String> {
        // Write message to temp file for {message_file} placeholder
        let msg_file =
            std::env::temp_dir().join(format!("pgit-submit-msg-{}.txt", std::process::id()));
        std::fs::write(&msg_file, body)?;

        let cmd = self
            .cmd_template
            .replace("{hash}", hash)
            .replace("{subject}", &shell_escape(subject))
            .replace("{message}", &shell_escape(body))
            .replace("{message_file}", &msg_file.display().to_string());

        let branch = repo.get_current_branch()?;
        repo.git_pub(&["checkout", "--quiet", hash])?;

        let result = Command::new("sh")
            .current_dir(&repo.workdir)
            .args(["-c", &cmd])
            .status();

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);
        let _ = std::fs::remove_file(&msg_file);

        match result {
            Ok(s) if s.success() => Ok("Custom command completed successfully.".to_string()),
            Ok(_) => Ok("Custom command exited with error.".to_string()),
            Err(e) => Err(color_eyre::eyre::eyre!("Failed to run command: {}", e)),
        }
    }

    fn update(&self, repo: &Repo, hash: &str, subject: &str, base: &str) -> Result<String> {
        self.submit(repo, hash, subject, base, "")
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        true
    }

    fn mark_submitted(&self, _repo: &Repo, _patches: &mut [PatchEntry]) {}

    fn sync(
        &self,
        _repo: &Repo,
        _patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Custom commands don't support sync — re-submit individually with 'p'.");
        Ok(Vec::new())
    }
}

/// Escape a string for safe use in sh -c by wrapping in single quotes.
/// Any single quotes in the input are replaced with '\'' (end quote, escaped quote, start quote).
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
