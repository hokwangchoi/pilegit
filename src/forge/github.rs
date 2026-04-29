use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

/// If `gh` reports the repo default branch, return a ref that exists locally (`origin/<name>` first,
/// then `<name>`). Called from [`crate::forge::stack_base_hint::try_from_forge_cli`] for [`crate::forge::ForgeKind::GitHub`].
pub(crate) fn try_cli_default_stack_base(repo: &Repo) -> Option<String> {
    let output = Command::new("gh")
        .current_dir(&repo.workdir)
        .args([
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "--jq",
            ".defaultBranchRef.name",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() || name == "null" {
        return None;
    }
    let origin_ref = format!("origin/{}", name);
    if repo
        .git_pub(&["rev-parse", "--verify", "--quiet", &origin_ref])
        .is_ok()
    {
        return Some(origin_ref);
    }
    if repo
        .git_pub(&["rev-parse", "--verify", "--quiet", &name])
        .is_ok()
    {
        return Some(name);
    }
    None
}

pub struct GitHub;

impl Forge for GitHub {
    fn name(&self) -> &str {
        "GitHub"
    }

    fn submit(
        &self,
        repo: &Repo,
        hash: &str,
        subject: &str,
        base: &str,
        body: &str,
    ) -> Result<String> {
        let branch = repo.get_current_branch()?;
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.force_update_and_push(&branch_name, hash)?;

        let create = Command::new("gh")
            .current_dir(&repo.workdir)
            .args([
                "pr",
                "create",
                "--head",
                &branch_name,
                "--base",
                base,
                "--title",
                subject,
                "--body",
                body,
            ])
            .output()?;

        let _ = repo.git_pub(&["checkout", "--quiet", &branch]);

        if create.status.success() {
            let url = String::from_utf8_lossy(&create.stdout).trim().to_string();
            return Ok(format!("PR created: {} → {}", url, base));
        }

        let stderr = String::from_utf8_lossy(&create.stderr);
        if stderr.contains("already exists") {
            self.edit_base(repo, &branch_name, base);
            return Ok(format!("PR updated: {} → {}", branch_name, base));
        }

        Err(eyre!("gh pr create failed: {}", stderr))
    }

    fn update(&self, repo: &Repo, hash: &str, subject: &str, base: &str) -> Result<String> {
        let _ = repo.fetch_origin();
        let branch_name = repo.make_pgit_branch_name(subject);

        let _ = repo.force_update_and_push(&branch_name, hash);

        self.edit_base(repo, &branch_name, base);
        Ok(format!("PR updated: {} → {}", branch_name, base))
    }

    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool) {
        let (full, available) = self.list_open_full(repo);
        let map = full.into_iter().map(|(k, (num, _url))| (k, num)).collect();
        (map, available)
    }

    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool {
        // Look up PR number first — more reliable
        let number = self.get_pr_number(repo, branch);
        let target = number.as_deref().unwrap_or(branch);

        let result = Command::new("gh")
            .current_dir(&repo.workdir)
            .args(["pr", "edit", target, "--base", base])
            .stderr(std::process::Stdio::null())
            .output();

        if let Ok(ref out) = result {
            if out.status.success() {
                return true;
            }
        }

        // Fallback: REST API
        if let Some(n) = &number {
            let api_path = format!("repos/{{owner}}/{{repo}}/pulls/{}", n);
            let base_field = format!("base={}", base);
            return Command::new("gh")
                .current_dir(&repo.workdir)
                .args(["api", "-X", "PATCH", &api_path, "-f", &base_field])
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        }
        false
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        let (pr_map, gh_available) = self.list_open_full(repo);
        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if gh_available {
                if let Some((pr_num, pr_url)) = pr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(*pr_num);
                    patch.pr_url = Some(pr_url.clone());
                }
            } else if repo.git_pub(&["rev-parse", "--verify", &branch]).is_ok() {
                patch.status = PatchStatus::Submitted;
                patch.pr_branch = Some(branch);
            }
        }
    }

    fn sync(
        &self,
        repo: &Repo,
        patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>> {
        on_progress("Fetching latest from origin...");
        let _ = repo.fetch_origin();

        let base = repo.base()?;
        let base_branch = base.strip_prefix("origin/").unwrap_or(&base).to_string();

        on_progress("Checking open PRs on GitHub...");
        let (open_prs, _) = self.list_open(repo);
        let mut updates = Vec::new();

        for (i, patch) in patches.iter().enumerate() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if !open_prs.contains_key(&branch) {
                continue;
            }

            on_progress(&format!("Syncing: {} ...", &patch.subject));

            let correct_base = if i == 0 {
                base_branch.clone()
            } else {
                let mut base_for_pr = base_branch.clone();
                for j in (0..i).rev() {
                    let parent = &patches[j];
                    let parent_branch = repo.make_pgit_branch_name(&parent.subject);
                    if open_prs.contains_key(&parent_branch) {
                        let _ = repo.git_pub(&["branch", "-f", &parent_branch, &parent.hash]);
                        let _ = repo.git_pub(&["push", "-f", "origin", &parent_branch]);
                        base_for_pr = parent_branch;
                        break;
                    }
                }
                base_for_pr
            };

            let _ = repo.git_pub(&["branch", "-f", &branch, &patch.hash]);
            let _ = repo.git_pub(&["push", "-f", "origin", &branch]);

            let edited = self.edit_base(repo, &branch, &correct_base);
            let status = if edited { "✓" } else { "⚠" };
            updates.push(format!("{} {} → {}", status, branch, correct_base));
        }

        Ok(updates)
    }

    fn pr_description_draft_hint(&self, repo: &Repo, _subject: &str) -> Option<String> {
        crate::forge::pr_description::github_conventional_templates(repo)
    }
}

impl GitHub {
    /// Fetch open PRs with full data (number + URL).
    fn list_open_full(&self, repo: &Repo) -> (HashMap<String, (u32, String)>, bool) {
        let mut map = HashMap::new();
        let output = Command::new("gh")
            .current_dir(&repo.workdir)
            .args([
                "pr",
                "list",
                "--state",
                "open",
                "--json",
                "number,headRefName,url",
                "--limit",
                "100",
            ])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let json = String::from_utf8_lossy(&out.stdout);
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                    for pr in prs {
                        if let (Some(num), Some(head)) =
                            (pr["number"].as_u64(), pr["headRefName"].as_str())
                        {
                            if head.starts_with("pgit/") {
                                let url = pr["url"].as_str().unwrap_or("").to_string();
                                map.insert(head.to_string(), (num as u32, url));
                            }
                        }
                    }
                }
                (map, true)
            }
            _ => (map, false),
        }
    }

    fn get_pr_number(&self, repo: &Repo, branch: &str) -> Option<String> {
        let output = Command::new("gh")
            .current_dir(&repo.workdir)
            .args(["pr", "view", branch, "--json", "number", "-q", ".number"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            let num = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !num.is_empty() {
                Some(num)
            } else {
                None
            }
        } else {
            None
        }
    }
}
