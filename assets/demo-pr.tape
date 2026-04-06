# pilegit PR flow demo — requires a real GitHub repo
#
# Before recording:
#   1. Clone your actual repo (or any repo you own on GitHub)
#   2. Create a few test commits on a branch
#   3. Make sure `gh auth status` works
#   4. Make sure .pilegit.toml exists with type = "github"
#   5. Adjust the sleep timings — network calls vary
#
# Run:
#   cd /path/to/your/repo
#   vhs /path/to/demo-pr.tape

Output demo-pr.gif

Set Shell "bash"
Set FontSize 14
Set Width 1000
Set Height 600
Set Padding 16
Set Framerate 30
Set TypingSpeed 40ms
Set Theme "Catppuccin Mocha"

# Wait for shell to initialize
Sleep 1s

Env EDITOR "nano"

# ── Launch ──────────────────────────────────────────────

Type "pgit"
Sleep 500ms
Enter
Sleep 2s

# ── Submit bottom commit as PR ──────────────────────────

# Move to first commit
Type "g"
Sleep 500ms

# Press p to submit
Type "p"
Sleep 1.5s

# nano opens for PR description — write something and save
Type "First PR in the stack: database migrations"
Sleep 800ms
Ctrl+o
Sleep 800ms
Enter
Sleep 800ms
Ctrl+x
Sleep 8s
# Wait for gh pr create to finish, show result
Enter
Sleep 2s

# ── Submit second commit ────────────────────────────────

Type "j"
Sleep 500ms
Type "p"
Sleep 1.5s
Type "Second PR: auth middleware (stacked on first)"
Sleep 800ms
Ctrl+o
Sleep 800ms
Enter
Sleep 800ms
Ctrl+x
Sleep 8s
Enter
Sleep 2s

# ── Sync all PRs ────────────────────────────────────────

Type "s"
Sleep 10s
Enter
Sleep 2s

# ── Rebase onto latest base ────────────────────────────

Type "r"
Sleep 500ms
Type "y"
Sleep 8s
Enter
Sleep 2s

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms

# ── Show PRs in terminal ───────────────────────────────

Sleep 500ms
Type "gh pr list"
Sleep 500ms
Enter
Sleep 3s
