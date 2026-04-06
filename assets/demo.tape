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
# Display layout (newest at top, .rev() rendering):
#   ┌ [4] feat: dashboard endpoint     ← cursor starts here (index 4)
#   │ [3] feat: rate limiting
#   │ [2] feat: profile API
#   │ [1] feat: auth middleware
#   └ [0] feat: add database schema    ← oldest, closest to base
#
# Controls: k=up(cursor+1), j=down(cursor-1), g=top(len-1), G=bottom(0)
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
# Cursor starts at index 4 (top = newest = dashboard)

Type "pgit"
Sleep 500ms
Enter
Sleep 2s

# ── Navigate down the stack ─────────────────────────────
# j=down (cursor-1): 4 → 3 → 2 → 1

Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 600ms

# Cursor at 1 (auth middleware). Go back up one.
# k=up (cursor+1): 1 → 2
Type "k"
Sleep 600ms

# Cursor at 2 (profile API)

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

# ── Squash: auth middleware + profile API ────────────────
# Cursor at 2 (profile API).
# V starts selection at 2.
# j extends down to 1 (auth middleware).
# Selection = indices 1-2.

Type "V"
Sleep 500ms
Type "j"
Sleep 500ms

# s shows confirm: "Squash 2 commits? (y/n)"
Type "s"
Sleep 800ms

# y confirms → TUI suspends → box appears → nano opens
Type "y"
Sleep 2s

# nano shows commit messages. Just save as-is.
Ctrl+o
Sleep 1s
Enter
Sleep 1s
Ctrl+x
Sleep 5s

# ── Reorder ─────────────────────────────────────────────
# After squash, 4 commits remain. Cursor at 1 (squashed commit).
# k=up: 1 → 2 (rate limiting)
# Ctrl+k moves it up (swaps with 3 = dashboard)

Type "k"
Sleep 500ms
Ctrl+k
Sleep 1.5s

# ── Submit PRs from bottom (oldest, closest to base) ────
# G = bottom = index 0 (database schema)

Type "G"
Sleep 500ms
Type "p"
Sleep 2s

# nano opens with PR template. Just save default.
Ctrl+o
Sleep 800ms
Enter
Sleep 800ms
Ctrl+x
Sleep 8s
Enter
Sleep 2s

# ── Submit next commit ──────────────────────────────────
# k=up: 0 → 1 (squashed auth + profile commit)

Type "k"
Sleep 500ms
Type "p"
Sleep 2s

# Save default template
Ctrl+o
Sleep 800ms
Enter
Sleep 800ms
Ctrl+x
Sleep 6.5s
Enter
Sleep 2s

# ── Rebase (fetches + rebases + syncs PRs) ──────────────
# r shows confirm: "Rebase onto origin/main? (y/n)"

Type "r"
Sleep 800ms
Type "y"
Sleep 10s
Enter
Sleep 2s

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms

# ── Show PRs in terminal ───────────────────────────────

Type "gh pr list"
Sleep 500ms
Enter
Sleep 3s
