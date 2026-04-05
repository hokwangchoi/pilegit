use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

pub struct Gitea;

impl Forge for Gitea {
    fn name(&self) -> &str { "Gitea" }

    fn submit(
        &self, repo: &Repo, hash: &str, subject: &str,
        base: &str, body: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        let create = Command::new("tea")
            .current_dir(&repo.workdir)
            .args(["pr", "create",
                "--head", &branch_name, "--base", base,
                "--title", subject, "--description", body])
            .output()?;

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        if create.status.success() {
            let out = String::from_utf8_lossy(&create.stdout).trim().to_string();
            return Ok(format!("PR created: {}", out));
        }

        let stderr = String::from_utf8_lossy(&create.stderr);
        Err(eyre!("tea pr create failed: {}", stderr))
    }

    fn update(
        &self, repo: &Repo, hash: &str, subject: &str, base: &str,
    ) -> Result<String> {
        let _ = repo.fetch_origin();
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        self.edit_base(repo, &branch_name, base);
        Ok(format!("PR updated: {} → {}", branch_name, base))
    }

    fn list_open(&self, _repo: &Repo) -> (HashMap<String, u32>, bool) {
        // tea doesn't have great JSON output — fall back to local branch check
        (HashMap::new(), false)
    }

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        // tea CLI doesn't support editing PR base easily
        false
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if repo.git_pub(&["rev-parse", "--verify", &branch]).is_ok() {
                patch.status = PatchStatus::Submitted;
                patch.pr_branch = Some(branch);
            }
        }
    }

    fn sync(
        &self, repo: &Repo, patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Fetching latest from origin...");
        let _ = repo.fetch_origin();

        let mut updates = Vec::new();
        for patch in patches {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if repo.git_pub(&["rev-parse", "--verify", &branch]).is_ok() {
                on_progress(&format!("Pushing: {} ...", &patch.subject));
                let _ = repo.git_pub(&["branch", "-f", &branch, &patch.hash]);
                let _ = repo.git_pub(&["push", "-f", "origin", &branch]);
                updates.push(format!("✓ {} pushed", branch));
            }
        }
        Ok(updates)
    }
}
