pub mod github;
pub mod gitlab;
pub mod gitea;
pub mod phabricator;
pub mod custom;

use std::collections::HashMap;

use color_eyre::Result;

use crate::core::config::Config;
use crate::core::stack::PatchEntry;
use crate::git::ops::Repo;

/// Trait for code review platform integrations.
pub trait Forge {
    /// Submit a new PR/MR/revision for a commit.
    fn submit(
        &self, repo: &Repo, hash: &str, subject: &str,
        base: &str, body: &str,
    ) -> Result<String>;

    /// Update an existing PR/MR/revision (force-push + update base).
    fn update(
        &self, repo: &Repo, hash: &str, subject: &str, base: &str,
    ) -> Result<String>;

    /// List open PRs/MRs for the user's pgit branches.
    /// Returns (branch_name → number, whether the CLI is available).
    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool);

    /// Edit the base/target branch of a PR/MR.
    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool;

    /// Mark submitted patches based on open reviews.
    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]);

    /// Sync all submitted reviews: force-push + update bases.
    fn sync(
        &self, repo: &Repo, patches: &[PatchEntry],
        on_progress: &dyn Fn(&str),
    ) -> Result<Vec<String>>;

    /// Whether this forge uses named branches for PRs.
    fn uses_branches(&self) -> bool { true }

    /// Whether pilegit should open an editor for the description before submit.
    /// Platforms like Phabricator have their own editor flow.
    fn needs_description_editor(&self) -> bool { true }

    /// Display name of the platform.
    fn name(&self) -> &str;
}

/// Create the appropriate Forge based on config.
pub fn create_forge(config: &Config) -> Box<dyn Forge> {
    match config.forge.forge_type.as_str() {
        "github" => Box::new(github::GitHub),
        "gitlab" => Box::new(gitlab::GitLab),
        "gitea" => Box::new(gitea::Gitea),
        "phabricator" => Box::new(phabricator::Phabricator),
        "custom" => Box::new(custom::Custom::new(
            config.forge.submit_cmd.clone().unwrap_or_default(),
        )),
        _ => Box::new(github::GitHub),
    }
}
