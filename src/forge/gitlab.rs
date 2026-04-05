use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

pub struct GitLab;

impl Forge for GitLab {
    fn name(&self) -> &str { "GitLab" }

    fn submit(
        &self, repo: &Repo, hash: &str, subject: &str,
        base: &str, body: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        let create = Command::new("glab")
            .current_dir(&repo.workdir)
            .args(["mr", "create",
                "--head", &branch_name, "--base", base,
                "--title", subject, "--description", body,
                "--yes"])
            .output()?;

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        if create.status.success() {
            let url = String::from_utf8_lossy(&create.stdout).trim().to_string();
            return Ok(format!("MR created: {}", url));
        }

        let stderr = String::from_utf8_lossy(&create.stderr);
        if stderr.contains("already exists") {
            self.edit_base(repo, &branch_name, base);
            return Ok(format!("MR updated: {} → {}", branch_name, base));
        }

        Err(eyre!("glab mr create failed: {}", stderr))
    }

    fn update(
        &self, repo: &Repo, hash: &str, subject: &str, base: &str,
    ) -> Result<String> {
        let _ = repo.fetch_origin();
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        self.edit_base(repo, &branch_name, base);
        Ok(format!("MR updated: {} → {}", branch_name, base))
    }

    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool) {
        let mut map = HashMap::new();
        let output = Command::new("glab")
            .current_dir(&repo.workdir)
            .args(["mr", "list", "--mine", "--json", "iid,sourceBranch"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let json = String::from_utf8_lossy(&out.stdout);
                if let Ok(mrs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                    for mr in mrs {
                        if let (Some(num), Some(head)) = (
                            mr["iid"].as_u64(), mr["sourceBranch"].as_str(),
                        ) {
                            if head.starts_with("pgit/") {
                                map.insert(head.to_string(), num as u32);
                            }
                        }
                    }
                }
                (map, true)
            }
            _ => (map, false),
        }
    }

    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool {
        Command::new("glab")
            .current_dir(&repo.workdir)
            .args(["mr", "update", branch, "--target-branch", base])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        let (mr_map, available) = self.list_open(repo);
        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if available {
                if let Some(&mr_num) = mr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(mr_num);
                }
            } else if repo.git_pub(&["rev-parse", "--verify", &branch]).is_ok() {
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

        let base = repo.detect_base()?;
        let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();

        on_progress("Checking open MRs on GitLab...");
        let (open_mrs, _) = self.list_open(repo);
        let mut updates = Vec::new();

        for (i, patch) in patches.iter().enumerate() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if !open_mrs.contains_key(&branch) { continue; }

            on_progress(&format!("Syncing: {} ...", &patch.subject));

            let correct_base = repo.walk_stack_for_base(patches, i, &open_mrs, &base_branch);

            let _ = repo.git_pub(&["branch", "-f", &branch, &patch.hash]);
            let _ = repo.git_pub(&["push", "-f", "origin", &branch]);

            let edited = self.edit_base(repo, &branch, &correct_base);
            let status = if edited { "✓" } else { "⚠" };
            updates.push(format!("{} {} → {}", status, branch, correct_base));
        }

        Ok(updates)
    }
}
