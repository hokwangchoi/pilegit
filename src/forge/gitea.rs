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
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        let create = Command::new("tea")
            .current_dir(&repo.workdir)
            .args(["pr", "create",
                "--head", &branch_name, "--base", base,
                "--title", subject, "--description", body])
            .output();

        match create {
            Ok(out) if out.status.success() => {
                let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Ok(format!("PR created: {}", url))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Err(eyre!("tea pr create failed: {}", stderr))
            }
            Err(e) => Err(eyre!("tea not found: {}", e)),
        }
    }

    fn update(
        &self, repo: &Repo, hash: &str, subject: &str, _base: &str,
    ) -> Result<String> {
        let _ = repo.fetch_origin();
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.git_pub(&["branch", "-f", &branch_name, hash])?;
        repo.git_pub(&["push", "-f", "origin", &branch_name])?;

        Ok(format!("PR pushed: {} (update base manually on Gitea if needed)", branch_name))
    }

    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool) {
        let mut map = HashMap::new();
        // tea pulls list with JSON output
        let output = Command::new("tea")
            .current_dir(&repo.workdir)
            .args(["pr", "list", "--output", "json",
                "--fields", "index,head,state"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let json = String::from_utf8_lossy(&out.stdout);
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                    for pr in prs {
                        let state = pr["state"].as_str().unwrap_or("");
                        if state != "open" { continue; }

                        let num = pr["index"].as_u64()
                            .or_else(|| pr["number"].as_u64());
                        // head can be a string or an object with a "name" field
                        let head = pr["head"].as_str()
                            .or_else(|| pr["head"]["name"].as_str())
                            .or_else(|| pr["head"]["ref"].as_str());

                        if let (Some(num), Some(head)) = (num, head) {
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

    fn edit_base(&self, _repo: &Repo, _branch: &str, _base: &str) -> bool {
        // tea CLI doesn't support editing PR base directly.
        // Gitea users should update base via the web UI.
        false
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        let (pr_map, available) = self.list_open(repo);
        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if available {
                if let Some(&pr_num) = pr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(pr_num);
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

        let (open_prs, _) = self.list_open(repo);
        let mut updates = Vec::new();

        for patch in patches {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if !open_prs.contains_key(&branch) { continue; }

            on_progress(&format!("Pushing: {} ...", &patch.subject));
            let _ = repo.git_pub(&["branch", "-f", &branch, &patch.hash]);
            let _ = repo.git_pub(&["push", "-f", "origin", &branch]);
            updates.push(format!("✓ {} pushed (update base on Gitea web if needed)", branch));
        }
        Ok(updates)
    }
}
