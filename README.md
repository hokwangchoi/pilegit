# pilegit (`pgit`)

**Git stacking with style** — manage, squash, reorder, and submit PRs from an interactive TUI.

pilegit treats your branch as a *pile* of commits. You develop on a single branch, making logical commits, then use the TUI to organize them into reviewable chunks, submit stacked PRs, and handle rebasing — all with full undo history that actually restores git state.

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
| `j` / `↓` | Move cursor down (toward older) |
| `k` / `↑` | Move cursor up (toward newer) |
| `g` / `G` | Jump to top (newest) / bottom (oldest) |
| `Enter` / `Space` | Expand/collapse commit details |
| `d` | View full diff of commit |
| `V` | Enter visual select mode |
| `Shift+↑` / `Shift+↓` | Start selection and extend |
| `Ctrl+↑` / `Ctrl+↓` | Reorder patch (modifies git history) |
| `Ctrl+k` / `Ctrl+j` | Reorder patch (alternative) |
| `e` | Edit/amend commit at cursor |
| `i` | Insert new commit (choose location) |
| `x` | Remove commit from git history |
| `r` | Rebase stack onto base branch |
| `p` | Submit/publish commit as PR |
| `u` | Undo (restores git state) |
| `Ctrl+r` | Redo (restores git state) |
| `h` | View undo/redo history |
| `?` | Show full help screen |
| `q` | Quit |

### Select Mode

| Key | Action |
|---|---|
| `j` / `k` / `↑` / `↓` | Extend selection |
| `Shift+↑` / `Shift+↓` | Extend selection |
| `s` | Squash selected commits |
| `Esc` / `q` | Cancel selection |

### Diff View

| Key | Action |
|---|---|
| `j` / `k` | Scroll line by line |
| `Ctrl+d` / `Ctrl+u` | Scroll half-page |
| `q` / `Esc` | Back to stack view |

## Submitting PRs

Press `p` to submit the commit at your cursor as a PR. pilegit supports two modes:

### GitHub (default)

If no `PGIT_SUBMIT_CMD` is set, pilegit uses the `gh` CLI to create proper stacked PRs:

- Creates a branch `pgit/<hash>-<subject>` for the selected commit
- If there's a commit below it in the stack, creates a branch for that too and sets it as the PR base — so the PR only shows **one commit's diff**
- If it's the bottom of the stack, the PR base is `main`
- Pushes both branches and creates the PR via `gh pr create`
- You stay on your original branch the entire time

**Prerequisites:** Install the [GitHub CLI](https://cli.github.com/) and run `gh auth login`.

**Updating a PR:** Just press `p` again on the same commit after making changes (via `e`). pilegit force-pushes the branch and the PR updates automatically.

### Custom Command (Phabricator, Gerrit, etc.)

Set `PGIT_SUBMIT_CMD` to use any review tool. pilegit checks out the target commit, runs the command, then returns to your branch.

```bash
# Phabricator — each commit becomes a Differential revision
export PGIT_SUBMIT_CMD="arc diff HEAD^"

# Gerrit — push to the review ref
export PGIT_SUBMIT_CMD="git push origin HEAD:refs/for/main"

# Custom script with placeholders
export PGIT_SUBMIT_CMD="my-tool submit --hash {hash} --title '{subject}'"
```

**Placeholders:** `{hash}` and `{subject}` are replaced with the commit's values.

Add to your shell config (`~/.zshrc`, `~/.bashrc`) to persist.

## Edit Commit

Press `e` to edit/amend the commit at cursor:

1. pilegit starts an interactive rebase and pauses at the selected commit
2. The working tree has the state of that commit — edit your code
3. Press `Enter` when done
4. pilegit automatically runs `git add -A` + `git commit --amend --no-edit`
5. Then rebases the remaining commits on top
6. If conflicts arise, enter the conflict resolution flow

## Insert Commit

Press `i`, then choose:
- `a` — Insert after the cursor position (uses `git rebase -i` with a break point)
- `t` — Insert at the top of the stack (just commit at HEAD)

Make your changes, `git add` + `git commit`, press `Enter`. pilegit rebases the rest.

## Squash Commits

1. Select commits with `V` + `j`/`k` (or `Shift+↑↓`)
2. Press `s`, confirm with `y`
3. Your editor opens with the combined commit message
4. Save and close — pilegit runs `git rebase -i` with the actual squash
5. Stack reloads from git with the squashed commit

## Rebase

Press `r` to rebase the entire stack onto the base branch. If conflicts occur:

1. pilegit shows conflicting files
2. Resolve conflicts in your editor, then `git add` the resolved files
3. Press `c` to continue — if no conflicts remain, it finishes automatically
4. Press `a` to abort

## Undo / Redo

Every operation (remove, reorder, squash, rebase, edit, insert) is recorded in the undo timeline with a descriptive message and the git HEAD hash.

- `u` — undo: runs `git reset --hard` to restore the previous git state
- `Ctrl+r` — redo: advances to the next state
- `h` — view the full undo/redo history with timestamps and descriptions

## Architecture

```
src/
├── main.rs          # CLI entry (clap) — routes to TUI or subcommands
├── core/
│   ├── stack.rs     # Stack data model (patches, squash, reorder, insert, drop)
│   └── history.rs   # Undo/redo timeline with git HEAD hash tracking
├── git/
│   └── ops.rs       # Git operations (rebase, squash, swap, remove, submit)
├── tui/
│   ├── mod.rs       # Terminal setup + suspend/resume handlers
│   ├── app.rs       # App state machine (modes, cursor, actions)
│   ├── input.rs     # Keybinding dispatch per mode
│   └── ui.rs        # Ratatui rendering (stack, diff, history, help, dialogs)
└── forge/
    └── mod.rs       # Reserved for extended forge integrations
```

## Design Philosophy

- **Single-branch workflow**: Develop on one branch, organize commits into logical PRs after the fact
- **Text-editor feel**: Navigate and manipulate commits like lines in an editor
- **Real git operations**: Every action (reorder, remove, squash, edit) modifies actual git history
- **Full undo**: Every operation is recorded with the git HEAD hash — undo restores the real state
- **Conflict-aware**: Reorder, remove, and squash detect and handle conflicts inline
- **PR-native**: Submit stacked PRs per commit, each showing only its own diff

## Roadmap

- [x] TUI with commit list, visual selection, diff viewer
- [x] Reorder commits in git history with conflict detection
- [x] Remove commits from git history with conflict detection
- [x] Squash commits in git with custom message editing
- [x] Edit/amend any commit with automatic rebase
- [x] Insert commits at any position via rebase break
- [x] Rebase stack onto base branch with conflict resolution
- [x] Undo/redo that restores actual git state
- [x] GitHub stacked PRs via `gh` CLI (one commit = one PR)
- [x] Custom submit command for Phabricator, Gerrit, etc.
- [ ] Config file (`.pilegit.toml`) for base branch, submit settings
- [ ] Commit message editing inline (without squash)
- [ ] Bulk submit all commits in the stack

## License

MIT
