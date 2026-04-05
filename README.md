# pilegit (`pgit`)

**Git stacking with style** — manage, squash, reorder, and submit PRs from an interactive TUI.

pilegit treats your branch as a *pile* of commits. You develop on a single branch, making logical commits, then use the TUI to organize them into reviewable chunks, submit stacked PRs, and handle rebasing — all with full undo history.

## Install

```bash
cargo install --path .
```

This installs the `pgit` binary.

Testing edit

more edit testing

testing edit at the bottom
## Quick Start

```bash
# Launch the interactive TUI (default)
pgit

# Or explicitly
pgit tui

# Non-interactive: show the current stack
pgit status
```

## TUI Keybindings

### Normal Mode

| Key | Action |
|---|---|
| `j` / `↓` | Move cursor down (toward older) |
| `k` / `↑` | Move cursor up (toward newer) |
| `g` / `G` | Jump to top (newest) / bottom (oldest) |
| `Enter` / `Space` | Expand/collapse commit details |
| `d` | View full diff of commit |
| `V` | Enter visual select mode |
| `Shift+↑` / `Shift+↓` | Start selection and extend |
| `K` / `J` | Move patch up/down (reorder stack) |
| `Alt+↑` / `Alt+↓` | Move patch up/down (reorder stack) |
| `i` | Insert new commit (suspends TUI) |
| `x` | Drop commit at cursor |
| `R` | Rebase stack onto base branch |
| `S` | Submit commit via custom command |
| `u` | Undo last operation |
| `Ctrl+r` | Redo |
| `h` | View undo history |
| `q` | Quit |

### Select Mode

| Key | Action |
|---|---|
| `j` / `k` / `↑` / `↓` | Extend selection |
| `s` | Squash selected commits |
| `Esc` / `q` | Cancel selection |

### Diff View

| Key | Action |
|---|---|
| `j` / `k` | Scroll line by line |
| `Ctrl+d` / `Ctrl+u` | Scroll half-page |
| `q` / `Esc` | Back to stack view |

## Custom Submit Command

Set the `PGIT_SUBMIT_CMD` environment variable to define how `S` submits a commit:

```bash
# Phabricator
export PGIT_SUBMIT_CMD="arc diff HEAD^"

# GitHub CLI
export PGIT_SUBMIT_CMD="gh pr create --head {hash} --title '{subject}'"

# Any custom script
export PGIT_SUBMIT_CMD="my-submit-tool --commit {hash}"
```

Placeholders `{hash}` and `{subject}` are replaced with the commit's values.

## Rebase

Press `R` to rebase the entire stack onto the base branch. If conflicts occur:

1. pilegit suspends the TUI and shows conflicting files
2. Resolve conflicts in your editor, then `git add` the resolved files
3. Press `c` to continue the rebase, or `a` to abort
4. Repeat until all conflicts are resolved
5. pilegit resumes with the updated stack

## Insert Commit

Press `i` to insert a new commit:

1. pilegit suspends the TUI
2. Make your changes and `git commit` as usual
3. Press `Enter` to return to pilegit
4. The stack refreshes with the new commit at HEAD
5. Use `K`/`J` to reorder it to the desired position

## Architecture

```
src/
├── main.rs          # CLI entry (clap) — routes to TUI or subcommands
├── core/
│   ├── stack.rs     # Stack data model (patches, squash, reorder, insert, drop)
│   └── history.rs   # Undo/redo via state snapshots
├── git/
│   └── ops.rs       # Git operations (shells out to git for prototype)
├── tui/
│   ├── mod.rs       # Terminal setup/teardown
│   ├── app.rs       # App state machine (modes, cursor, actions)
│   ├── input.rs     # Keybinding dispatch per mode
│   └── ui.rs        # Ratatui rendering (stack view, diff view, history, dialogs)
└── forge/
    └── mod.rs       # Future: GitHub/GitLab PR submission
```

## Design Philosophy

- **Single-branch workflow**: Develop on one branch, organize commits into logical PRs after the fact
- **Text-editor feel**: Navigate and manipulate commits like lines in an editor
- **Full undo**: Every destructive operation is snapshotted — go back anytime
- **Conflict-aware**: Check for conflicts before and after reordering (planned)
- **PR-native**: Submit stacked PRs directly from the TUI (planned)

## Roadmap

- [x] Core stack model with squash, reorder, drop, insert
- [x] Undo/redo history (state-timeline model)
- [x] TUI with commit list, navigation, visual selection
- [x] Correct visual direction (newest=top, j=down, k=up)
- [x] Diff viewer with syntax coloring
- [x] Confirm dialogs for destructive ops
- [x] Insert commit with TUI suspend/resume
- [x] Rebase onto base branch with conflict handling
- [x] Custom submit command via `PGIT_SUBMIT_CMD`
- [ ] Actual git rebase execution for squash/reorder (currently in-memory)
- [ ] Conflict detection before reordering (dry-run)
- [ ] GitHub PR submission via API (stacked PRs)
- [ ] Config file (`.pilegit.toml`) for base branch, submit command
- [ ] Commit message editing inline

## License

MIT
