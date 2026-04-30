//! Open a [`Repo`](super::ops::Repo) with stack base resolved from `.pilegit.toml` or auto-detection.

use color_eyre::Result;

use crate::core::config::{Config, ForgeConfig, RepoConfig};
use super::ops::Repo;

/// Open the current git repo and resolve the stack base from config (if any) or heuristics.
pub fn open_resolved() -> Result<Repo> {
    let repo = Repo::open()?;
    let config = Config::load(&repo.workdir).unwrap_or_else(|| Config {
        forge: ForgeConfig {
            forge_type: "github".to_string(),
            submit_cmd: None,
        },
        repo: RepoConfig::default(),
    });
    let base = repo.resolve_base(config.repo.base.as_deref())?;
    Ok(repo.with_resolved_base(base))
}
