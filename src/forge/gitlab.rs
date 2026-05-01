use std::collections::HashMap;
use std::process::Command;

use color_eyre::{eyre::eyre, Result};

use super::Forge;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

pub struct GitLab;

impl Forge for GitLab {
    fn name(&self) -> &str {
        "GitLab"
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

        let create = Command::new("glab")
            .current_dir(&repo.workdir)
            .args([
                "mr",
                "create",
                "--source-branch",
                &branch_name,
                "--target-branch",
                base,
                "--title",
                subject,
                "--description",
                body,
                "--yes",
            ])
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

    fn update(&self, repo: &Repo, hash: &str, subject: &str, base: &str) -> Result<String> {
        let _ = repo.fetch_origin();
        let branch_name = repo.make_pgit_branch_name(subject);

        let _ = repo.force_update_and_push(&branch_name, hash);

        self.edit_base(repo, &branch_name, base);
        Ok(format!("MR updated: {} → {}", branch_name, base))
    }

    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool) {
        let (full, available) = self.list_open_full(repo);
        let map = full.into_iter().map(|(k, (num, _url))| (k, num)).collect();
        (map, available)
    }

    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool {
        // glab mr update requires the MR IID, not branch name
        let iid = self.get_mr_iid(repo, branch);
        let target = match &iid {
            Some(n) => n.as_str(),
            None => branch, // fallback: glab tries current branch's MR
        };

        Command::new("glab")
            .current_dir(&repo.workdir)
            .args(["mr", "update", target, "--target-branch", base])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]) {
        let (mr_map, available) = self.list_open_full(repo);
        for patch in patches.iter_mut() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if available {
                if let Some((mr_num, mr_url)) = mr_map.get(&branch) {
                    patch.status = PatchStatus::Submitted;
                    patch.pr_branch = Some(branch);
                    patch.pr_number = Some(*mr_num);
                    patch.pr_url = Some(mr_url.clone());
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

        on_progress("Checking open MRs on GitLab...");
        let (open_mrs, _) = self.list_open(repo);
        let mut updates = Vec::new();

        for (i, patch) in patches.iter().enumerate() {
            let branch = repo.make_pgit_branch_name(&patch.subject);
            if !open_mrs.contains_key(&branch) {
                continue;
            }

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

    fn pr_description_draft_hint(&self, repo: &Repo, _subject: &str) -> Option<String> {
        crate::forge::pr_description::gitlab_conventional_templates(repo)
    }
}

impl GitLab {
    /// Fetch open MRs with full data (IID + URL).
    /// Parses text output of `glab mr list`. When stdout is piped
    /// (as with Command::output), glab disables the pager and outputs
    /// full-width untruncated text.
    fn list_open_full(&self, repo: &Repo) -> (HashMap<String, (u32, String)>, bool) {
        let mut map = HashMap::new();

        let output = Command::new("glab")
            .current_dir(&repo.workdir)
            .args(["mr", "list", "-P", "100"])
            .env("PAGER", "cat")
            .env("GLAB_PAGER", "")
            .env("NO_COLOR", "1")
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let base_url = self.get_project_url(repo);

                for line in text.lines() {
                    let clean = strip_ansi(line).trim().to_string();
                    if !clean.starts_with('!') {
                        continue;
                    }

                    // Extract IID from !<number>
                    let iid: u32 = match clean
                        .strip_prefix('!')
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                    {
                        Some(n) => n,
                        None => continue,
                    };

                    // Format: !N  project!N  title  (target) ← (source)
                    // Source branch (ours) is AFTER ←.
                    // If no ←, take the last pgit/ branch in the line.
                    let source = if let Some(arrow_pos) = clean.find('←') {
                        let after_arrow = &clean[arrow_pos..];
                        after_arrow
                            .split_whitespace()
                            .map(|w| w.trim_matches(|c: char| c == '(' || c == ')'))
                            .find(|w| w.starts_with("pgit/"))
                    } else {
                        // No arrow — find the last pgit/ branch
                        clean
                            .split_whitespace()
                            .map(|w| w.trim_matches(|c: char| c == '(' || c == ')'))
                            .rfind(|w| w.starts_with("pgit/"))
                    };

                    if let Some(branch) = source {
                        let url = match &base_url {
                            Some(u) => format!("{}/-/merge_requests/{}", u, iid),
                            None => String::new(),
                        };
                        map.insert(branch.to_string(), (iid, url));
                    }
                }
                (map, true)
            }
            _ => (map, false),
        }
    }

    /// Get the project web URL from the git remote.
    fn get_project_url(&self, repo: &Repo) -> Option<String> {
        let remote = repo.git_pub(&["remote", "get-url", "origin"]).ok()?;
        let remote = remote.trim();
        // git@gitlab.com:user/repo.git → https://gitlab.com/user/repo
        if remote.starts_with("git@") {
            let rest = remote.strip_prefix("git@")?;
            let (host, path) = rest.split_once(':')?;
            let path = path.strip_suffix(".git").unwrap_or(path);
            Some(format!("https://{}/{}", host, path))
        } else {
            // https://gitlab.com/user/repo.git → https://gitlab.com/user/repo
            Some(remote.strip_suffix(".git").unwrap_or(remote).to_string())
        }
    }

    /// Look up the MR IID for a source branch.
    fn get_mr_iid(&self, repo: &Repo, branch: &str) -> Option<String> {
        let (mrs, available) = self.list_open_full(repo);
        if !available {
            return None;
        }
        mrs.get(branch).map(|(iid, _)| iid.to_string())
    }
}

/// Strip ANSI escape codes from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            result.push(c);
        }
    }
    result
}
