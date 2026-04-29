//! Initial text for the PR/MR description editor: config path, forge-specific discovery, then builtin.

use std::fs;
use std::path::PathBuf;

use crate::core::config::RepoConfig;
use crate::git::ops::Repo;

use super::Forge;

/// Standard GitHub-style template locations (single-file; see GitHub docs for PR templates).
const GITHUB_TEMPLATE_PATHS: &[&str] = &[
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
    "pull_request_template.md",
];

/// First existing non-empty file under `repo.workdir` from `relative_paths`, in order.
pub(crate) fn read_first_repo_template(repo: &Repo, relative_paths: &[&str]) -> Option<String> {
    for rel in relative_paths {
        let path = repo.workdir.join(rel);
        if let Ok(s) = fs::read_to_string(&path) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

pub(crate) fn github_conventional_templates(repo: &Repo) -> Option<String> {
    read_first_repo_template(repo, GITHUB_TEMPLATE_PATHS)
}

pub(crate) fn gitlab_conventional_templates(repo: &Repo) -> Option<String> {
    let dir = repo.workdir.join(".gitlab/merge_request_templates");
    let entries = fs::read_dir(&dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ex| ex == "md"))
        .collect();
    if files.is_empty() {
        return None;
    }
    files.sort();
    let default_idx = files.iter().position(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("Default.md"))
    });
    let mut ordered = Vec::new();
    if let Some(i) = default_idx {
        ordered.push(files.remove(i));
    }
    ordered.extend(files);
    for p in ordered {
        if let Ok(s) = fs::read_to_string(&p) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn substitute_subject(template: &str, subject: &str) -> String {
    if template.contains("{{subject}}") {
        template.replace("{{subject}}", subject)
    } else {
        template.to_string()
    }
}

fn builtin_fallback(subject: &str) -> String {
    format!("## Description\n\n{}\n\n## Test Plan\n\n\n", subject)
}

/// Editor seed for a new PR/MR: `[repo].pr_description_template` file, then
/// [`Forge::pr_description_draft_hint`], then a small built-in outline.
pub fn compose_initial_draft(
    repo: &Repo,
    forge: &dyn Forge,
    repo_cfg: &RepoConfig,
    subject: &str,
) -> String {
    if let Some(rel) = repo_cfg.pr_description_template.as_ref() {
        let path = repo.workdir.join(rel);
        if let Ok(s) = fs::read_to_string(&path) {
            if !s.trim().is_empty() {
                return substitute_subject(&s, subject);
            }
        }
    }
    if let Some(h) = forge.pr_description_draft_hint(repo, subject) {
        return substitute_subject(&h, subject);
    }
    builtin_fallback(subject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::github::GitHub;

    #[test]
    fn github_reads_dot_github_lowercase() {
        let dir =
            std::env::temp_dir().join(format!("pgit-pr-template-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".github")).unwrap();
        std::fs::write(
            dir.join(".github/pull_request_template.md"),
            "## Checklist\n\n- [ ] tests\n",
        )
        .unwrap();
        let repo = Repo::at_dir(dir.clone());
        let got = github_conventional_templates(&repo).unwrap();
        assert!(got.contains("Checklist"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_prefers_config_path_over_forge() {
        let dir = std::env::temp_dir().join(format!("pgit-pr-compose-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".github")).unwrap();
        std::fs::write(dir.join(".github/pull_request_template.md"), "FORGE").unwrap();
        std::fs::write(dir.join("custom.md"), "CONFIG {{subject}}").unwrap();
        let repo = Repo::at_dir(dir.clone());
        let cfg = RepoConfig {
            base: None,
            pr_description_template: Some("custom.md".to_string()),
        };
        let draft = compose_initial_draft(&repo, &GitHub, &cfg, "my feat");
        assert_eq!(draft, "CONFIG my feat");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_falls_back_to_builtin() {
        let dir =
            std::env::temp_dir().join(format!("pgit-pr-fallback-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repo::at_dir(dir.clone());
        let cfg = RepoConfig::default();
        let draft = compose_initial_draft(&repo, &GitHub, &cfg, "only subject");
        assert!(draft.contains("only subject"));
        assert!(draft.contains("Test Plan"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
