pub mod custom;
pub mod custom;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod phabricator;
pub mod pr_description;
pub mod stack_base_hint;

use std::collections::HashMap;

/// Forge platform from `.pilegit.toml` `[forge] type = ...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForgeKind {
    GitHub,
    GitLab,
    Gitea,
    Phabricator,
    Custom,
}

impl ForgeKind {
    /// Parse from config `type` string. Unknown values match [`create_forge`]: treated as GitHub.
    pub fn from_config_str(s: &str) -> Self {
        match s {
            "github" => Self::GitHub,
            "gitlab" => Self::GitLab,
            "gitea" => Self::Gitea,
            "phabricator" => Self::Phabricator,
            "custom" => Self::Custom,
            _ => Self::GitHub,
        }
    }
}

use color_eyre::Result;

use crate::core::config::Config;
use crate::core::stack::{PatchEntry, PatchStatus};
use crate::git::ops::Repo;

/// Trait for code review platform integrations.
pub trait Forge {
    /// Submit a new PR/MR/revision for a commit.
    fn submit(
        &self,
        repo: &Repo,
        hash: &str,
        subject: &str,
        base: &str,
        body: &str,
    ) -> Result<String>;

    /// Update an existing PR/MR/revision (force-push + update base).
    fn update(&self, repo: &Repo, hash: &str, subject: &str, base: &str) -> Result<String>;

    /// List open PRs/MRs for the user's pgit branches.
    /// Returns (branch_name → number, whether the CLI is available).
    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool);

    /// Edit the base/target branch of a PR/MR.
    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool;

    /// Mark submitted patches based on open reviews.
    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]);

    /// Sync all submitted reviews: force-push + update bases.
    fn sync(
        &self,
        repo: &Repo,
        patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>>;

    /// Whether pilegit should open an editor for the description before submit.
    /// Platforms like Phabricator have their own editor flow.
    fn needs_description_editor(&self) -> bool {
        true
    }

    /// Optional seed for the PR/MR description editor (host template files, API, etc.).
    /// Runs after `[repo].pr_description_template` if that file is missing or empty.
    /// Use `None` to rely on config + built-in outline. Implement per forge in
    /// [`crate::forge::pr_description`] helpers where conventions exist (GitHub, GitLab, …).
    fn pr_description_draft_hint(&self, _repo: &Repo, _subject: &str) -> Option<String> {
        None
    }

    /// Extract trailers from a commit body that should be preserved during squash.
    /// Each forge knows its own trailer format (e.g. "Differential Revision:" for
    /// Phabricator, "Change-Id:" for Gerrit). Default: none.
    fn get_trailers(&self, _body: &str) -> Vec<String> {
        Vec::new()
    }

    /// Find dependency trailers across all commits in the stack.
    /// Called after rebase or before sync when commit order may have changed.
    /// Default: no-op. Phabricator uses this to update "Depends on DXXX".
    fn fix_dependencies(&self, _repo: &Repo) -> Result<()> {
        Ok(())
    }

    /// Detect forge-specific stale branches that aren't caught by ancestor checks.
    /// E.g. Phabricator's `arc land` squashes commits into a new hash but
    /// preserves the `Differential Revision:` trailer — this method matches
    /// branches by trailer against the base branch history.
    /// Default: no-op (returns empty).
    fn find_landed_branches(&self, _repo: &Repo, _branches: &[String]) -> Vec<String> {
        Vec::new()
    }

    /// Check if any submitted PRs have been updated on the remote by someone else.
    /// Returns a list of (branch_name, description) for diverged PRs.
    /// Compares origin/<branch> hash against what pgit last pushed (stored in
    /// .git/pgit-sync-state.json). This correctly ignores local edits — editing
    /// a commit changes the stack hash but the saved hash still matches remote.
    fn check_diverged(&self, repo: &Repo, patches: &[PatchEntry]) -> Vec<(String, String)> {
        let mut diverged = Vec::new();
        let _ = repo.fetch_origin();
        let saved = repo.read_sync_state();

        for patch in patches {
            if patch.status != PatchStatus::Submitted {
                continue;
            }
            let branch = repo.make_pgit_branch_name(&patch.subject);
            let remote = format!("origin/{}", branch);

            // Get remote branch hash
            let remote_hash = match repo.git_pub(&["rev-parse", &remote]) {
                Ok(h) => h.trim().to_string(),
                Err(_) => continue, // no remote branch yet
            };

            // Get what pgit last pushed for this branch
            let saved_hash = match saved.get(&branch) {
                Some(h) => h.clone(),
                None => continue, // no saved state → first push
            };

            // If remote differs from what pgit last pushed → someone else changed it
            if remote_hash != saved_hash {
                diverged.push((
                    branch.clone(),
                    format!("Remote has newer changes for '{}'", patch.subject),
                ));
            }
        }
        diverged
    }

    /// Get the remote ref to merge for a diverged branch.
    /// For GitHub/GitLab/Gitea: returns origin/<branch>.
    /// For Phabricator: arc patches the revision onto a temp ref.
    /// Returns None if not diverged or not applicable.
    fn get_remote_ref(&self, repo: &Repo, patch: &PatchEntry) -> Option<String> {
        let branch = repo.make_pgit_branch_name(&patch.subject);
        let remote = format!("origin/{}", branch);
        if repo.git_pub(&["rev-parse", "--verify", &remote]).is_ok() {
            Some(remote)
        } else {
            None
        }
    }

    /// Save sync state after a successful push.
    /// Stores the hash we pushed for each branch in .git/pgit-sync-state.json.
    fn save_sync_state(&self, repo: &Repo, patches: &[PatchEntry]) {
        let mut state = repo.read_sync_state();
        for patch in patches {
            if patch.status != PatchStatus::Submitted {
                continue;
            }
            let branch = repo.make_pgit_branch_name(&patch.subject);
            let remote = format!("origin/{}", branch);
            if let Ok(hash) = repo.git_pub(&["rev-parse", &remote]) {
                state.insert(branch, hash.trim().to_string());
            }
        }
        repo.write_sync_state(&state);
    }

    /// Display name of the platform.
    fn name(&self) -> &str;
}

/// Create the appropriate Forge based on config.
pub fn create_forge(config: &Config) -> Box<dyn Forge> {
    match ForgeKind::from_config_str(config.forge.forge_type.as_str()) {
        ForgeKind::GitHub => Box::new(github::GitHub),
        ForgeKind::GitLab => Box::new(gitlab::GitLab),
        ForgeKind::Gitea => Box::new(gitea::Gitea),
        ForgeKind::Phabricator => Box::new(phabricator::Phabricator),
        ForgeKind::Custom => Box::new(custom::Custom::new(
            config.forge.submit_cmd.clone().unwrap_or_default(),
        )),
    }
}
