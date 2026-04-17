use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

pub struct Gitea;

/// Extract "owner/repo" from a git remote URL.
/// Handles: git@host:owner/repo.git, https://host/owner/repo.git,
///          ssh://git@host/owner/repo.git
fn repo_slug(repo: &Repo) -> Option<String> {
    let url = repo.git_pub(&["remote", "get-url", "origin"]).ok()?;
    let url = url.trim();

    // git@host:owner/repo.git or ssh://git@host/owner/repo.git
    let path = if let Some(pos) = url.find(':') {
        if url.starts_with("ssh://") || url.starts_with("http") {
            // ssh://git@host/owner/repo or https://host/owner/repo
            url.split('/').skip(3).collect::<Vec<_>>().join("/")
        } else {
            // git@host:owner/repo
            url[pos + 1..].to_string()
        }
    } else {
        return None;
    };

    let slug = path.trim_end_matches(".git").to_string();
    if slug.contains('/') { Some(slug) } else { None }
}

/// Extract base URL from tea's login config (the actual web/API URL).
/// Falls back to parsing the git remote if tea isn't available.
fn base_url(repo: &Repo) -> Option<String> {
    // Try tea login list first — this has the correct web URL
    let output = Command::new("tea")
        .current_dir(&repo.workdir)
        .args(["login", "list", "--output", "json"])
        .output()
        .ok()?;
    if output.status.success() {
        let json = String::from_utf8_lossy(&output.stdout);
        if let Ok(logins) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            // Use the default login, or the first one
            for login in &logins {
                let is_default = login["default"].as_bool().unwrap_or(false);
                if is_default {
                    if let Some(url) = login["url"].as_str() {
                        return Some(url.trim_end_matches('/').to_string());
                    }
                }
            }
            // No default found — use first login
            if let Some(login) = logins.first() {
                if let Some(url) = login["url"].as_str() {
                    return Some(url.trim_end_matches('/').to_string());
                }
            }
        }
    }

    // Fallback: parse from git remote (only works if SSH host == web host)
    let url = repo.git_pub(&["remote", "get-url", "origin"]).ok()?;
    let url = url.trim();
    if url.starts_with("https://") || url.starts_with("http://") {
        let parts: Vec<&str> = url.splitn(4, '/').collect();
        if parts.len() >= 3 {
            return Some(format!("{}//{}", parts[0], parts[2]));
        }
    }
    None
}

impl Forge for Gitea {
    fn name(&self) -> &str { "Gitea" }

    fn submit(
        &self, repo: &Repo, hash: &str, subject: &str,
        base: &str, body: &str,
    ) -> Result<String> {
        let slug = repo_slug(repo)
            .ok_or_else(|| eyre!("Could not detect owner/repo from git remote"))?;
        let branch_name = repo.make_pgit_branch_name(subject);

        repo.force_update_and_push(&branch_name, hash)?;

        let create = Command::new("tea")
            .current_dir(&repo.workdir)
            .args(["pr", "create",
                "--repo", &slug,
                "--head", &branch_name, "--base", base,
                "--title", subject, "--description", body])
            .output();

        match create {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // tea outputs the PR URL on success
                if stdout.is_empty() {
                    // Construct URL ourselves
                    if let Some(url) = base_url(repo) {
                        Ok(format!("PR created: {}/{}/pulls", url, slug))
                    } else {
                        Ok(format!("PR created: {}", branch_name))
                    }
                } else {
                    Ok(format!("PR created: {}", stdout))
                }
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

        let _ = repo.force_update_and_push(&branch_name, hash);

        Ok(format!("PR pushed: {}", branch_name))
    }

    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool) {
        let mut map = HashMap::new();
        let slug = match repo_slug(repo) {
            Some(s) => s,
            None => return (map, false),
        };

        let output = Command::new("tea")
            .current_dir(&repo.workdir)
            .args(["pr", "list", "--repo", &slug,
                "--output", "json", "--fields", "index,head,state"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let json = String::from_utf8_lossy(&out.stdout);
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                    for pr in prs {
                        let state = pr["state"].as_str().unwrap_or("");
                        if state != "open" { continue; }

                        let num = pr["index"].as_u64()
                            .or_else(|| pr["number"].as_u64())
                            // tea returns numbers as strings
                            .or_else(|| pr["index"].as_str()?.parse::<u64>().ok())
                            .or_else(|| pr["number"].as_str()?.parse::<u64>().ok());
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
        let url_base = base_url(repo);
        let slug = repo_slug(repo);

        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if available {
                if let Some(&pr_num) = pr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(pr_num);
                    // Construct PR URL
                    if let (Some(ref url), Some(ref s)) = (&url_base, &slug) {
                        patch.pr_url = Some(format!("{}/{}/pulls/{}", url, s, pr_num));
                    }
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
            updates.push(format!("✓ {} pushed", branch));
        }
        Ok(updates)
    }
}
