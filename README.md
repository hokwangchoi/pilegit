<h1 align="center">pilegit (<code>pgit</code>)</h1>

<p align="center">
  <strong>Git stacking with style</strong> — manage, squash, reorder, and submit stacked PRs from an interactive TUI.
</p>

<!-- Replace OWNER with your GitHub username -->
<p align="center">
  <a href="https://github.com/OWNER/pilegit/stargazers"><img src="https://img.shields.io/github/stars/OWNER/pilegit?style=flat&color=yellow" alt="Stars"></a>
  <a href="https://github.com/OWNER/pilegit/network/members"><img src="https://img.shields.io/github/forks/OWNER/pilegit?style=flat&color=blue" alt="Forks"></a>
  <a href="https://github.com/OWNER/pilegit/blob/main/LICENSE"><img src="https://img.shields.io/github/license/OWNER/pilegit?style=flat" alt="License"></a>
  <a href="https://github.com/OWNER/pilegit/releases"><img src="https://img.shields.io/github/v/release/OWNER/pilegit?style=flat&color=green" alt="Release"></a>
</p>

<!-- Demo gif: record with `vhs` or `asciinema` — see "Recording a Demo" section below -->
<p align="center">
  <img src="assets/demo.gif" alt="pilegit demo" width="720">
</p>

---

pilegit treats your branch as a *pile* of commits. Develop on a single branch, make logical commits, then use the TUI to organize them — reorder, squash, edit, insert, remove — and submit each as a stacked PR. Full undo history restores actual git state. Works with GitHub, GitLab, Gitea, Phabricator, and custom commands.

## Install

```bash
# From source (requires Rust 1.75+)
cargo install --path .

# Or build and copy the binary
cargo build --release
cp target/release/pgit ~/.local/bin/
```

<!-- Uncomment when published to crates.io -->
<!-- ```bash -->
<!-- cargo install pilegit -->
<!-- ``` -->

## Quick Start

```bash
cd your-repo

# Launch the TUI (auto-prompts setup on first run)
pgit

# Non-interactive: show the current stack
pgit status

# Re-run setup anytime
pgit init
```

## What It Looks Like

```
  pilegit — my-feature (5 commits on origin/main)

    ○ a1b2c3d feat: add dashboard page
    ○ b2c3d4e feat: user profile endpoint
  ◈ c3d4e5f feat: auth middleware        PR#14
       hokwang • 2026-04-05
       branch: pgit/hokwang/feat-auth-middleware
       https://github.com/user/repo/pull/14
  ◈ d4e5f6a feat: database migrations    PR#12
  → e5f6a7b feat: initial project setup  PR#11

  ▸ Rebase completed. Stack: 5 commits.

  ↑k/↓j:move  V:select  Ctrl+↑↓:reorder  e:edit  p:submit  s:sync  ?:help
```

- `→` marks the cursor, `◈` marks submitted PRs
- Expand any commit with `Enter` to see author, branch, PR URL, and body
- PR URLs are clickable in most terminals (Cmd/Ctrl-click)

## Core Workflow

```
1. Write code normally, making small logical commits
2. pgit              ← launch the TUI
3. Ctrl+↑/↓          ← reorder commits into the right sequence
4. V + j/k + s       ← select and squash related commits
5. p                  ← submit commit as a stacked PR
6. e                  ← edit a commit, auto-amend + rebase
7. p                  ← update the existing PR (force-push)
8. r                  ← rebase onto latest main, sync PRs
```

## Keybindings

### Normal Mode

| Key | Action |
|---|---|
| `j`/`↓` `k`/`↑` | Move cursor |
| `g` / `G` | Jump to top / bottom |
| `Enter` / `Space` | Expand/collapse commit details |
| `d` | View full diff |
| `V` or `Shift+↑↓` | Start visual selection |
| `Ctrl+↑↓` or `Ctrl+k/j` | Reorder commit (modifies git history) |
| `e` | Edit/amend commit |
| `i` | Insert new commit (after cursor or at top) |
| `x` | Remove commit from history |
| `r` | Rebase onto base branch + sync PRs |
| `p` | Submit or update PR for commit |
| `s` | Sync all submitted PRs |
| `u` / `Ctrl+r` | Undo / Redo (restores git state) |
| `h` | View undo/redo history |
| `?` | Full help screen |

### Select Mode

`V` or `Shift+↑↓` to start, `j`/`k` to extend, `s` to squash, `Esc` to cancel.

### Diff View

`j`/`k` to scroll, `Ctrl+d`/`Ctrl+u` for half-page, `q` to go back.

## Setup

On first run, pilegit prompts for your platform and base branch:

```
  ▸ pilegit setup

  Which code review platform do you use?

    1  GitHub      (uses gh CLI)
    2  GitLab      (uses glab CLI)
    3  Gitea       (uses tea CLI)
    4  Phabricator (uses arc CLI)
    5  Custom command

  Select [1-5]: 1
  Base branch detected: origin/main
  ✓ Config saved to .pilegit.toml
```

Config stored in `.pilegit.toml`:

```toml
[forge]
type = "github"

[repo]
base = "origin/main"
```

### Platform Prerequisites

| Platform | CLI Tool | Install |
|---|---|---|
| GitHub | [`gh`](https://cli.github.com/) | `brew install gh` then `gh auth login` |
| GitLab | [`glab`](https://gitlab.com/gitlab-org/cli) | `brew install glab` then `glab auth login` |
| Gitea | [`tea`](https://gitea.com/gitea/tea) | See Gitea docs |
| Phabricator | `arc` | `arc install-certificate` |
| Custom | Any shell command | Define during `pgit init` |

## Stacked PRs

Each commit becomes its own PR. pilegit manages the base branches so each PR shows **only its diff**:

```
  Stack:                    GitHub PRs:
  ┌ feat: dashboard         PR#15 → pgit/hokwang/feat-auth (base)
  │ feat: auth middleware    PR#14 → main (base, since PR#13 was merged)
  └ feat: migrations         ← merged, cleaned up
```

When a parent PR is merged, pilegit auto-detects this and updates the child's base to `main`.

Branch names include your git username (`pgit/<username>/<subject>`) so multiple team members can use pilegit on the same repo without conflicts.

## Architecture

```
src/
├── main.rs            # CLI entry — TUI, status, or init
├── core/
│   ├── config.rs      # .pilegit.toml config + setup wizard
│   ├── stack.rs       # Stack data model (patches)
│   └── history.rs     # Undo/redo timeline with git HEAD hash tracking
├── git/
│   └── ops.rs         # Git operations (rebase, squash, swap, remove, edit)
├── forge/
│   ├── mod.rs         # Forge trait + factory
│   ├── github.rs      # GitHub via gh CLI
│   ├── gitlab.rs      # GitLab via glab CLI
│   ├── gitea.rs       # Gitea via tea CLI
│   ├── phabricator.rs # Phabricator via arc CLI
│   └── custom.rs      # Custom command template
└── tui/
    ├── mod.rs         # Terminal setup + suspend/resume handlers
    ├── app.rs         # App state machine (modes, cursor, forge)
    ├── input.rs       # Keybinding dispatch per mode
    └── ui.rs          # Ratatui rendering
```

### Forge Trait

Adding a new platform means implementing one trait:

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
    fn uses_branches(&self) -> bool { true }
    fn name(&self) -> &str;
}
```

## How It Works Under the Hood

Every pilegit operation maps to real git commands:

| Action | Git Operation |
|---|---|
| Reorder | `git rebase -i` with sed to swap `pick` lines |
| Remove | `git rebase -i` changing `pick` to `drop` |
| Squash | `git rebase -i` with `pick` + `squash` markers |
| Edit | `git rebase -i` with `edit` marker, then `git commit --amend` |
| Insert | `git rebase -i` with `break` marker |
| Undo | `git reset --hard <previous-HEAD-hash>` |
| Submit | `git branch -f pgit/... <hash>` + `git push -f` + platform CLI |
| Rebase | `git fetch origin` + `git rebase origin/main` |

## Comparison

| Feature | pilegit | git-branchless | graphite | ghstack |
|---|---|---|---|---|
| Interactive TUI | ✓ | ✓ | – | – |
| Single-branch workflow | ✓ | ✓ | – | ✓ |
| Stacked PRs | ✓ | partial | ✓ | ✓ |
| Multi-platform | ✓ (5) | Git only | GitHub only | GitHub only |
| Undo/redo | ✓ (git state) | ✓ | – | – |
| No daemon | ✓ | – (needs hook) | – (needs service) | ✓ |
| Config needed | `.pilegit.toml` | `.git/` hooks | `.graphite/` | – |
| Language | Rust | Rust | TypeScript | Python |

## Roadmap

- [x] Interactive TUI with commit list, selection, diff viewer
- [x] Reorder, remove, squash, edit, insert commits
- [x] Full undo/redo with git state restoration
- [x] Stacked PRs with automatic base management
- [x] Multi-platform: GitHub, GitLab, Gitea, Phabricator, Custom
- [x] Multi-user safe branch naming
- [x] Config file with setup wizard
- [x] PR sync with stale branch cleanup
- [ ] Commit message editing inline
- [ ] Bulk submit all commits
- [ ] `cargo install pilegit` via crates.io
- [ ] Homebrew formula
- [ ] AUR package

## Recording a Demo

To create the `assets/demo.gif` shown at the top:

**Option A: [vhs](https://github.com/charmbracelet/vhs)** (recommended — scripted, reproducible)

```bash
brew install vhs
# Create a demo.tape script:
cat > demo.tape << 'EOF'
Output assets/demo.gif
Set FontSize 14
Set Width 960
Set Height 540
Set Theme "Catppuccin Mocha"

Type "pgit"
Enter
Sleep 2s
Down Down Down
Sleep 1s
Type "V"
Down
Sleep 500ms
Type "s"
Sleep 500ms
Type "y"
Sleep 2s
Type "q"
EOF
vhs demo.tape
```

**Option B: [asciinema](https://asciinema.org/) + [agg](https://github.com/asciinema/agg)**

```bash
asciinema rec demo.cast
# Use pgit normally, then exit
agg demo.cast assets/demo.gif
```

## License

MIT
