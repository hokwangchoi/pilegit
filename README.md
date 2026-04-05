# pilegit (`pgit`)

**Git stacking with style** — manage, squash, reorder, and submit stacked PRs from an interactive TUI.

<!-- Uncomment after creating the GitHub repo (replace OWNER with your username):
[![Stars](https://img.shields.io/github/stars/OWNER/pilegit?style=flat&color=yellow)](https://github.com/OWNER/pilegit/stargazers)
[![Forks](https://img.shields.io/github/forks/OWNER/pilegit?style=flat&color=blue)](https://github.com/OWNER/pilegit/network/members)
[![License](https://img.shields.io/github/license/OWNER/pilegit?style=flat)](LICENSE)
[![Release](https://img.shields.io/github/v/release/OWNER/pilegit?style=flat&color=green)](https://github.com/OWNER/pilegit/releases)
-->

<!-- Add a demo gif: `brew install vhs && vhs demo.tape` or `asciinema rec + agg` -->
<!-- <p align="center"><img src="assets/demo.gif" alt="demo" width="720"></p> -->

Develop on a single branch, organize commits into reviewable chunks, submit each as a stacked PR. Full undo restores actual git state. Works with GitHub, GitLab, Gitea, Phabricator, and custom commands.

## Install

```bash
cargo install --path .
```

## Quick Start

```bash
pgit          # launch TUI (prompts setup on first run)
pgit init     # re-run setup
pgit status   # show stack non-interactively
```

## What It Looks Like

```
  pilegit — my-feature (5 commits on origin/main)

    ○ a1b2c3d feat: add dashboard page
    ○ b2c3d4e feat: user profile endpoint
  ◈ c3d4e5f feat: auth middleware        PR#14
       hokwang • 2026-04-05
       https://github.com/user/repo/pull/14
  ◈ d4e5f6a feat: database migrations    PR#12
  → e5f6a7b feat: initial project setup  PR#11

  ↑k/↓j:move  V:select  Ctrl+↑↓:reorder  e:edit  p:submit  s:sync  ?:help
```

`→` cursor · `◈` submitted PR · Expand with `Enter` for details + clickable PR URL

## Keybindings

| Key | Action |
|---|---|
| `j`/`↓` `k`/`↑` | Move cursor |
| `g` / `G` | Top / bottom |
| `Enter` | Expand/collapse commit |
| `d` | Full diff view |
| `V` or `Shift+↑↓` | Visual selection |
| `Ctrl+↑↓` | Reorder commit (real `git rebase -i`) |
| `e` | Edit/amend commit |
| `i` | Insert new commit |
| `x` | Remove commit |
| `r` | Rebase onto base + sync PRs |
| `p` | Submit / update PR |
| `s` | Sync all submitted PRs |
| `u` / `Ctrl+r` | Undo / redo (restores git state) |
| `?` | Help |
| `q` | Quit |

**Select mode:** `V` to start → `j`/`k` extend → `s` squash → `Esc` cancel

## Setup

First run prompts you for platform and base branch. Saved to `.pilegit.toml`:

```toml
[forge]
type = "github"    # github | gitlab | gitea | phabricator | custom

[repo]
base = "origin/main"
```

| Platform | CLI | Install |
|---|---|---|
| GitHub | `gh` | [cli.github.com](https://cli.github.com/) |
| GitLab | `glab` | [gitlab.com/gitlab-org/cli](https://gitlab.com/gitlab-org/cli) |
| Gitea | `tea` | [gitea.com/gitea/tea](https://gitea.com/gitea/tea) |
| Phabricator | `arc` | `arc install-certificate` |
| Custom | any | Shell command with `{hash}`, `{subject}` placeholders |

## Stacked PRs

Each commit → one PR. pilegit manages base branches so each PR shows only its diff:

```
Stack:                       PRs on GitHub:
┌ feat: dashboard            PR#15 base=pgit/.../feat-auth
│ feat: auth middleware       PR#14 base=main (parent merged)
└ feat: migrations            ← merged, branch cleaned up
```

Branch naming: `pgit/<username>/<subject>` — multi-user safe.

Press `s` to sync: force-push all branches, update bases, prompt to clean up stale branches.

## Under the Hood

| Action | Git Operation |
|---|---|
| Reorder | `git rebase -i` with sed |
| Remove | `git rebase -i` → `drop` |
| Squash | `git rebase -i` → `pick` + `squash` |
| Edit | `git rebase -i` → `edit` + `commit --amend` |
| Undo | `git reset --hard <saved-HEAD>` |
| Submit | `git branch -f` + `git push -f` + CLI |
| Rebase | `git fetch origin` + `git rebase origin/main` |

## Forge Trait

Adding a new platform = implementing one trait:

```rust
pub trait Forge {
    fn submit(&self, repo: &Repo, hash: &str, subject: &str,
              base: &str, body: &str) -> Result<String>;
    fn update(&self, repo: &Repo, hash: &str, subject: &str,
              base: &str) -> Result<String>;
    fn list_open(&self, repo: &Repo) -> (HashMap<String, u32>, bool);
    fn edit_base(&self, repo: &Repo, branch: &str, base: &str) -> bool;
    fn mark_submitted(&self, repo: &Repo, patches: &mut [PatchEntry]);
    fn sync(&self, repo: &Repo, patches: &[PatchEntry],
            on_progress: &dyn Fn(&str)) -> Result<Vec<String>>;
    fn name(&self) -> &str;
}
```

## Comparison

| | pilegit | git-branchless | graphite | ghstack |
|---|---|---|---|---|
| Interactive TUI | ✓ | ✓ | – | – |
| Single-branch | ✓ | ✓ | – | ✓ |
| Stacked PRs | ✓ | partial | ✓ | ✓ |
| Multi-platform | 5 | git only | GitHub | GitHub |
| Undo/redo | ✓ | ✓ | – | – |
| No daemon | ✓ | – | – | ✓ |
| Language | Rust | Rust | TypeScript | Python |

## Architecture

```
src/
├── main.rs            # CLI — TUI, status, init
├── core/
│   ├── config.rs      # .pilegit.toml + setup wizard
│   ├── stack.rs       # Stack data model
│   └── history.rs     # Undo/redo with HEAD hash tracking
├── git/
│   └── ops.rs         # Git operations (rebase, squash, swap, etc.)
├── forge/
│   ├── mod.rs         # Forge trait + factory
│   ├── github.rs      # GitHub (gh)
│   ├── gitlab.rs      # GitLab (glab)
│   ├── gitea.rs       # Gitea (tea)
│   ├── phabricator.rs # Phabricator (arc)
│   └── custom.rs      # Custom command
└── tui/
    ├── mod.rs         # Terminal + suspend/resume handlers
    ├── app.rs         # State machine (modes, cursor, forge)
    ├── input.rs       # Keybinding dispatch
    └── ui.rs          # Ratatui rendering
```

## Roadmap

- [x] TUI with commit list, selection, diff, scrolling
- [x] Reorder, remove, squash, edit, insert commits
- [x] Undo/redo with git state restoration
- [x] Stacked PRs with automatic base management
- [x] Multi-platform: GitHub, GitLab, Gitea, Phabricator, Custom
- [x] Multi-user safe branch naming
- [x] Config file with setup wizard
- [x] PR sync with stale branch cleanup
- [ ] Commit message editing inline
- [ ] Bulk submit all commits

## License

MIT
