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

# Set editor to nano
Type "export EDITOR=nano"
Sleep 300ms
Enter
Sleep 500ms

# ── Launch ──────────────────────────────────────────────

Type "pgit"
Sleep 500ms
Enter
Sleep 1.5s

# ── Submit bottom commit as PR ──────────────────────────

# Move to first commit
Type "g"
Sleep 400ms

# Press p to submit
Type "p"
Sleep 1s

# nano opens for PR description — write something and save
Type "First PR in the stack: database migrations"
Sleep 500ms
Ctrl+o
Sleep 500ms
Enter
Sleep 500ms
Ctrl+x
Sleep 4s
# Wait for gh pr create to finish
Enter
Sleep 1s

# ── Submit second commit ────────────────────────────────

Type "j"
Sleep 400ms
Type "p"
Sleep 1s
Type "Second PR: auth middleware (stacked on first)"
Sleep 500ms
Ctrl+o
Sleep 500ms
Enter
Sleep 500ms
Ctrl+x
Sleep 4s
Enter
Sleep 1s

# ── Sync all PRs ────────────────────────────────────────

Type "s"
Sleep 7s
Enter
Sleep 1s

# ── Rebase onto latest base ────────────────────────────

Type "r"
Sleep 400ms
Type "y"
Sleep 7s
Enter
Sleep 1s

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms

# ── Show PRs in terminal ───────────────────────────────

Sleep 500ms
Type "gh pr list"
Sleep 500ms
Enter
Sleep 2s
