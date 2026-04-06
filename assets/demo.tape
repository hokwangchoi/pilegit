# pilegit combined demo
#
# This demo requires a real GitHub repo with `gh` auth.
#
# Setup:
#   cd ~/code/pilegit
#   git checkout -b demo-recording
#   echo "test1" > test1.txt && git add -A && git commit -m "feat: add database schema"
#   echo "test2" > test2.txt && git add -A && git commit -m "feat: auth middleware"
#   echo "test3" > test3.txt && git add -A && git commit -m "feat: profile API"
#   echo "test4" > test4.txt && git add -A && git commit -m "feat: rate limiting"
#   echo "test5" > test5.txt && git add -A && git commit -m "feat: dashboard endpoint"
#   vhs assets/demo.tape
#
# Cleanup after recording:
#   gh pr list --state open --json number,headRefName -q '.[] | select(.headRefName | startswith("pgit/")) | .number' | xargs -I{} gh pr close {}
#   git branch -l 'pgit/*' | xargs -I{} git branch -D {}
#   git branch -l 'pgit/*' | xargs -I{} git push origin --delete {}
#   git checkout master && git branch -D demo-recording
#   git push origin --delete demo-recording 2>/dev/null
#   rm -f test*.txt

Output assets/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1400
Set Height 800
Set Padding 20
Set Framerate 30
Set TypingSpeed 40ms
Set Theme "Catppuccin Mocha"
Env EDITOR "nano"

# Wait for shell to initialize
Sleep 1s

# ── Launch ──────────────────────────────────────────────

Type "pgit"
Sleep 500ms
Enter
Sleep 2s

# ── Navigate the stack ──────────────────────────────────

Type "k"
Sleep 300ms
Type "k"
Sleep 300ms
Type "k"
Sleep 600ms

Type "j"
Sleep 300ms
Type "j"
Sleep 600ms

# ── Expand commit details ───────────────────────────────

Enter
Sleep 1.5s
Enter
Sleep 600ms

# ── Diff view ───────────────────────────────────────────

Type "d"
Sleep 1.2s
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 600ms
Type "q"
Sleep 800ms

# ── Squash two commits ──────────────────────────────────

# Move to auth middleware
Type "k"
Sleep 300ms
Type "k"
Sleep 400ms

# Select down to include profile API
Type "V"
Sleep 500ms
Type "j"
Sleep 500ms

# Squash
Type "s"
Sleep 1.5s

# nano — save default message
Ctrl+o
Sleep 800ms
Enter
Sleep 800ms
Ctrl+x
Sleep 3s

# ── Reorder a commit ────────────────────────────────────

Type "j"
Sleep 400ms
Ctrl+k
Sleep 1s
Ctrl+k
Sleep 1.5s

# ── Submit PRs from bottom of stack (closest to base) ───

# Go to bottom of list (oldest commit, closest to base)
Type "G"
Sleep 500ms
Type "p"
Sleep 1.5s

# nano — write PR description and save
Type "First PR: database schema"
Sleep 600ms
Ctrl+o
Sleep 600ms
Enter
Sleep 600ms
Ctrl+x
Sleep 8s
Enter
Sleep 1.5s

# ── Submit next commit (stacked on top of first PR) ─────

Type "k"
Sleep 500ms
Type "p"
Sleep 1.5s
Type "Second PR: auth middleware (stacked)"
Sleep 600ms
Ctrl+o
Sleep 600ms
Enter
Sleep 600ms
Ctrl+x
Sleep 6.5s
Enter
Sleep 1.5s

# ── Rebase (fetches + rebases + syncs PRs) ──────────────

Type "r"
Sleep 500ms
Type "y"
Sleep 8s
Enter
Sleep 1.5s

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms

# ── Show PRs in terminal ───────────────────────────────

Type "gh pr list"
Sleep 500ms
Enter
Sleep 3s
