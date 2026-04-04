# pilegit (`pgit`)

**Git stacking with style** — manage, squash, reorder, and submit PRs from an interactive TUI.

pilegit treats your branch as a *pile* of commits. You develop on a single branch, making logical commits, then use the TUI to organize them into reviewable chunks, submit stacked PRs, and handle rebasing — all with full undo history.

## Install

```bash
cargo install --path .
```

This installs the `pgit` binary.

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
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `g` / `G` | Jump to bottom / top of stack |
| `Enter` / `Space` | Expand/collapse commit details |
| `d` | View full diff of commit |
| `V` | Enter visual select mode |
| `Shift+↑` / `Shift+↓` | Start selection and extend |
| `K` / `J` | Move patch up/down (reorder) |
| `Alt+↑` / `Alt+↓` | Move patch up/down (reorder) |
| `i` | Insert new commit at cursor (WIP) |
| `x` | Drop commit at cursor |
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

- [x] Core stack model with squash, reorder, drop
- [x] Undo/redo history
- [x] TUI with commit list, navigation, selection
- [x] Diff viewer with syntax coloring
- [x] Confirm dialogs for destructive ops
- [ ] Shell suspend/resume for inserting commits
- [ ] Actual git rebase execution (currently models changes in-memory)
- [ ] Conflict detection (dry-run rebase in temp worktree)
- [ ] GitHub PR submission via API
- [ ] `pgit sync` — pull + rebase stack onto updated base
- [ ] Config file (`.pilegit.toml`) for base branch, PR defaults
- [ ] Commit message editing inline

## License

MIT
