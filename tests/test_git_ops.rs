use std::path::PathBuf;
use std::process::Command;

/// Helper: create a temp git repo with an initial commit on `main`
/// and a local "origin" remote so detect_base works.
fn setup_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("pgit-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {} failed: {}",
            args.join(" "), String::from_utf8_lossy(&out.stderr));
    };

    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Test User"]);
    git(&["config", "user.email", "test@example.com"]);

    // Initial commit on main
    std::fs::write(dir.join("README.md"), "# test\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "initial commit"]);

    // Set up origin pointing to self so detect_base finds origin/main
    git(&["remote", "add", "origin", dir.to_str().unwrap()]);
    git(&["fetch", "origin"]);

    dir
}

/// Helper: add a commit with a file change.
fn add_commit(dir: &PathBuf, filename: &str, content: &str, message: &str) {
    std::fs::write(dir.join(filename), content).unwrap();
    Command::new("git").current_dir(dir)
        .args(["add", "."]).output().unwrap();
    Command::new("git").current_dir(dir)
        .args(["commit", "-m", message]).output().unwrap();
}

fn open_repo(dir: &PathBuf) -> pilegit::git::ops::Repo {
    pilegit::git::ops::Repo::at_dir(dir.clone())
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

// --- Tests ---

#[test]
fn list_stack_commits_count_and_order() {
    let dir = setup_repo("list");
    add_commit(&dir, "a.txt", "a", "feat: add a");
    add_commit(&dir, "b.txt", "b", "feat: add b");
    add_commit(&dir, "c.txt", "c", "feat: add c");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();

    assert_eq!(commits.len(), 3);
    assert_eq!(commits[0].subject, "feat: add a");
    assert_eq!(commits[1].subject, "feat: add b");
    assert_eq!(commits[2].subject, "feat: add c");
    cleanup(&dir);
}

#[test]
fn swap_commits_changes_order() {
    let dir = setup_repo("swap");
    add_commit(&dir, "a.txt", "aaa\n", "feat: first");
    add_commit(&dir, "b.txt", "bbb\n", "feat: second");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 2);

    let hash_a = &commits[0].hash[..7];
    let hash_b = &commits[1].hash[..7];
    let ok = repo.swap_commits(hash_a, hash_b).unwrap();
    assert!(ok, "swap should succeed without conflicts");

    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits[0].subject, "feat: second");
    assert_eq!(commits[1].subject, "feat: first");
    cleanup(&dir);
}

#[test]
fn remove_commit_drops_it() {
    let dir = setup_repo("remove");
    add_commit(&dir, "a.txt", "a\n", "feat: keep");
    add_commit(&dir, "b.txt", "b\n", "feat: remove me");
    add_commit(&dir, "c.txt", "c\n", "feat: also keep");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    let hash_b = &commits[1].hash[..7];

    let ok = repo.remove_commit(hash_b).unwrap();
    assert!(ok, "remove should succeed without conflicts");

    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].subject, "feat: keep");
    assert_eq!(commits[1].subject, "feat: also keep");
    cleanup(&dir);
}

#[test]
fn squash_commits_merges_them() {
    let dir = setup_repo("squash");
    add_commit(&dir, "a.txt", "a\n", "feat: part one");
    add_commit(&dir, "b.txt", "b\n", "feat: part two");
    add_commit(&dir, "c.txt", "c\n", "feat: unrelated");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();

    let hash_a = commits[0].hash[..7].to_string();
    let hash_b = commits[1].hash[..7].to_string();

    let ok = repo.squash_commits_with_message(
        &[hash_a, hash_b], "feat: combined",
    ).unwrap();
    assert!(ok, "squash should succeed without conflicts");

    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].subject, "feat: combined");
    assert_eq!(commits[1].subject, "feat: unrelated");
    cleanup(&dir);
}

#[test]
fn branch_name_sanitized() {
    let dir = setup_repo("brname");
    let repo = open_repo(&dir);

    let name = repo.make_pgit_branch_name("feat: add login page!");
    assert!(name.starts_with("pgit/"));
    assert!(name.contains("test-user"));
    assert!(!name.contains(' '));
    assert!(!name.contains('!'));
    assert!(!name.contains(':'));
    assert_eq!(name, name.to_lowercase());
    cleanup(&dir);
}

#[test]
fn branch_name_truncated() {
    let dir = setup_repo("brtrunc");
    let repo = open_repo(&dir);

    let long = "a".repeat(100);
    let name = repo.make_pgit_branch_name(&long);
    let slug = name.rsplit('/').next().unwrap();
    assert!(slug.len() <= 50);
    cleanup(&dir);
}

#[test]
fn branch_name_stable() {
    let dir = setup_repo("brstable");
    let repo = open_repo(&dir);

    let a = repo.make_pgit_branch_name("feat: something");
    let b = repo.make_pgit_branch_name("feat: something");
    assert_eq!(a, b);
    cleanup(&dir);
}

#[test]
fn rebase_onto_base_succeeds() {
    let dir = setup_repo("rebase");
    add_commit(&dir, "a.txt", "a\n", "feat: work");

    let repo = open_repo(&dir);
    let ok = repo.rebase_onto_base(&|_| {}).unwrap();
    assert!(ok, "rebase should complete cleanly");
    cleanup(&dir);
}

#[test]
fn diff_returns_content() {
    let dir = setup_repo("diff");
    add_commit(&dir, "hello.txt", "hello world\n", "add hello");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    let diff = repo.diff_full(&commits[0].hash).unwrap();

    assert!(diff.contains("hello world"));
    assert!(diff.contains("+hello world"));
    cleanup(&dir);
}

#[test]
fn edit_then_reorder_preserves_content() {
    let dir = setup_repo("edit_reorder");
    add_commit(&dir, "a.txt", "original_a\n", "feat: add a");
    add_commit(&dir, "b.txt", "original_b\n", "feat: add b");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 2);

    // Edit commit A: change its content
    let hash_a = &commits[0].hash[..7];
    let paused = repo.rebase_edit_commit(hash_a).unwrap();
    assert!(!paused, "should pause at commit");

    // Amend the commit with new content
    std::fs::write(dir.join("a.txt"), "edited_a\n").unwrap();
    Command::new("git").current_dir(&dir)
        .args(["add", "-A"]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["commit", "--amend", "--no-edit"]).output().unwrap();

    let ok = repo.rebase_continue().unwrap();
    assert!(ok, "rebase continue should succeed");

    // Verify edit took effect
    let content = std::fs::read_to_string(dir.join("a.txt")).unwrap();
    assert_eq!(content, "edited_a\n");

    // Now reorder: swap A and B
    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 2);
    let hash_a = &commits[0].hash[..7];
    let hash_b = &commits[1].hash[..7];
    let ok = repo.swap_commits(hash_a, hash_b).unwrap();
    assert!(ok, "swap should succeed");

    // After swap: B is first (older), A is second (newer)
    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits[0].subject, "feat: add b");
    assert_eq!(commits[1].subject, "feat: add a");

    // The edited content should still be present
    let content = std::fs::read_to_string(dir.join("a.txt")).unwrap();
    assert_eq!(content, "edited_a\n");
    cleanup(&dir);
}

#[test]
fn swap_with_custom_abbrev() {
    let dir = setup_repo("swap_abbrev");
    // Set a non-default core.abbrev
    Command::new("git").current_dir(&dir)
        .args(["config", "core.abbrev", "10"]).output().unwrap();

    add_commit(&dir, "a.txt", "aaa\n", "feat: first");
    add_commit(&dir, "b.txt", "bbb\n", "feat: second");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    let hash_a = &commits[0].hash[..7];
    let hash_b = &commits[1].hash[..7];

    let ok = repo.swap_commits(hash_a, hash_b).unwrap();
    assert!(ok, "swap should succeed with non-default core.abbrev");

    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits[0].subject, "feat: second");
    assert_eq!(commits[1].subject, "feat: first");
    cleanup(&dir);
}

#[test]
fn edit_middle_commit_preserves_others() {
    let dir = setup_repo("edit_middle");
    add_commit(&dir, "a.txt", "aaa\n", "feat: first");
    add_commit(&dir, "b.txt", "bbb\n", "feat: second");
    add_commit(&dir, "c.txt", "ccc\n", "feat: third");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();

    // Edit the middle commit
    let hash_b = &commits[1].hash[..7];
    let paused = repo.rebase_edit_commit(hash_b).unwrap();
    assert!(!paused);

    std::fs::write(dir.join("b.txt"), "edited_b\n").unwrap();
    Command::new("git").current_dir(&dir)
        .args(["add", "-A"]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["commit", "--amend", "--no-edit"]).output().unwrap();

    let ok = repo.rebase_continue().unwrap();
    assert!(ok);

    // All three commits should exist with correct content
    let commits = repo.list_stack_commits().unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(commits[0].subject, "feat: first");
    assert_eq!(commits[1].subject, "feat: second");
    assert_eq!(commits[2].subject, "feat: third");

    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "aaa\n");
    assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "edited_b\n");
    assert_eq!(std::fs::read_to_string(dir.join("c.txt")).unwrap(), "ccc\n");
    cleanup(&dir);
}

#[test]
fn has_uncommitted_changes_ignores_pilegit_toml() {
    let dir = setup_repo("dirty_check");
    let repo = open_repo(&dir);

    // Clean state
    assert!(!repo.has_uncommitted_changes());

    // Only .pilegit.toml — should be ignored
    std::fs::write(dir.join(".pilegit.toml"), "[forge]\ntype = \"github\"\n").unwrap();
    assert!(!repo.has_uncommitted_changes());

    // Real dirty file — should be detected
    std::fs::write(dir.join("dirty.txt"), "uncommitted\n").unwrap();
    assert!(repo.has_uncommitted_changes());
    cleanup(&dir);
}

#[test]
fn find_stale_branches_detects_landed_via_trailer() {
    let dir = setup_repo("stale_trailer");

    // Create a commit with a Differential Revision trailer (simulates submitted diff)
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    Command::new("git").current_dir(&dir).args(["add", "."]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["commit", "-m", "feat: add a\n\nDifferential Revision: https://p.example.com/D12345"])
        .output().unwrap();

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    let original_hash = commits[0].hash.clone();

    Command::new("git").current_dir(&dir)
        .args(["config", "user.name", "test-user"]).output().unwrap();

    // Create a pgit branch pointing to the original commit
    let branch = repo.make_pgit_branch_name("feat: add a");
    Command::new("git").current_dir(&dir)
        .args(["branch", &branch, &original_hash]).output().unwrap();

    let forge = pilegit::forge::phabricator::Phabricator;
    use pilegit::forge::Forge;

    // Initially: trailer not in origin/main → not landed
    let landed = forge.find_landed_branches(&repo, &[branch.clone()]);
    assert!(landed.is_empty());

    // Simulate arc land: create a NEW commit with same trailer but different hash,
    // and update origin/main to point at it
    Command::new("git").current_dir(&dir)
        .args(["checkout", "--orphan", "landed"]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["reset", "--hard"]).output().unwrap();
    std::fs::write(dir.join("README.md"), "# test\n").unwrap();
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    Command::new("git").current_dir(&dir).args(["add", "."]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["commit", "-m", "feat: add a (landed)\n\nDifferential Revision: https://p.example.com/D12345"])
        .output().unwrap();
    let landed_hash = String::from_utf8(
        Command::new("git").current_dir(&dir)
            .args(["rev-parse", "HEAD"]).output().unwrap().stdout
    ).unwrap().trim().to_string();

    // Verify the hash actually changed (arc land squashes/rewrites)
    assert_ne!(original_hash, landed_hash, "landed commit should have different hash");

    Command::new("git").current_dir(&dir)
        .args(["update-ref", "refs/remotes/origin/main", &landed_hash])
        .output().unwrap();

    // Now the trailer matches a commit in origin/main → landed
    let landed = forge.find_landed_branches(&repo, &[branch.clone()]);
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0], branch);
    cleanup(&dir);
}

#[test]
fn find_stale_branches_detects_landed_commits() {
    use std::collections::HashMap;

    let dir = setup_repo("stale_landed");
    add_commit(&dir, "a.txt", "a\n", "feat: add a");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();

    // Configure git user for branch naming
    Command::new("git").current_dir(&dir)
        .args(["config", "user.name", "test-user"]).output().unwrap();

    // Create a pgit branch pointing to the commit
    let branch = repo.make_pgit_branch_name("feat: add a");
    Command::new("git").current_dir(&dir)
        .args(["branch", &branch, &commits[0].hash]).output().unwrap();

    // Initially: branch is NOT an ancestor of origin/main (the commit is ahead)
    // gh_available=false, no open PRs → branch should NOT be stale
    let stale = repo.find_stale_branches_with(&HashMap::new(), false);
    assert!(stale.is_empty(), "branch should not be stale before landing");

    // Simulate landing: update origin/main to point at our commit
    Command::new("git").current_dir(&dir)
        .args(["update-ref", "refs/remotes/origin/main", &commits[0].hash])
        .output().unwrap();

    // Now the branch's commit is reachable from origin/main → stale
    let stale = repo.find_stale_branches_with(&HashMap::new(), false);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], branch);
    cleanup(&dir);
}

#[test]
fn diverged_remote_detected() {
    let dir = setup_repo("diverged");
    add_commit(&dir, "a.txt", "a\n", "feat: add a");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();

    // Create a pgit branch and simulate initial push
    let branch = repo.make_pgit_branch_name("feat: add a");
    Command::new("git").current_dir(&dir)
        .args(["branch", &branch, &commits[0].hash]).output().unwrap();
    // Simulate origin/<branch> matching local (as if we just pushed)
    Command::new("git").current_dir(&dir)
        .args(["update-ref", &format!("refs/remotes/origin/{}", branch), &commits[0].hash])
        .output().unwrap();

    // Not diverged: origin/<branch> is ancestor of local hash
    let is_ancestor = repo.git_pub(&[
        "merge-base", "--is-ancestor",
        &format!("origin/{}", branch), &commits[0].hash
    ]).is_ok();
    assert!(is_ancestor, "should not be diverged after push");

    // Simulate teammate force-pushing a different commit to the remote branch
    add_commit(&dir, "b.txt", "b\n", "teammate: add b");
    let new_commits = repo.list_stack_commits().unwrap();
    let teammate_hash = &new_commits[1].hash;
    Command::new("git").current_dir(&dir)
        .args(["update-ref", &format!("refs/remotes/origin/{}", branch), teammate_hash])
        .output().unwrap();
    // Reset local back to original (teammate pushed, we didn't)
    Command::new("git").current_dir(&dir)
        .args(["reset", "--hard", &commits[0].hash]).output().unwrap();

    // Now diverged: origin/<branch> is NOT ancestor of our local commit
    let is_ancestor = repo.git_pub(&[
        "merge-base", "--is-ancestor",
        &format!("origin/{}", branch), &commits[0].hash
    ]).is_ok();
    assert!(!is_ancestor, "should be diverged after teammate pushed");
    cleanup(&dir);
}

#[test]
fn not_diverged_after_our_push() {
    let dir = setup_repo("not_diverged");
    add_commit(&dir, "a.txt", "a\n", "feat: add a");
    add_commit(&dir, "b.txt", "b\n", "feat: add b");

    let repo = open_repo(&dir);
    let commits = repo.list_stack_commits().unwrap();
    let latest_hash = &commits[1].hash;

    // Create branch and simulate push of latest
    let branch = repo.make_pgit_branch_name("feat: add a");
    Command::new("git").current_dir(&dir)
        .args(["branch", &branch, &commits[0].hash]).output().unwrap();
    Command::new("git").current_dir(&dir)
        .args(["update-ref", &format!("refs/remotes/origin/{}", branch), &commits[0].hash])
        .output().unwrap();

    // origin/<branch> == local commit → ancestor → not diverged
    let is_ancestor = repo.git_pub(&[
        "merge-base", "--is-ancestor",
        &format!("origin/{}", branch), &commits[0].hash
    ]).is_ok();
    assert!(is_ancestor, "should not be diverged when remote matches local");
    cleanup(&dir);
}
