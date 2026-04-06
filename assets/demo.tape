# pilegit core demo
#
# Setup:
#   bash demo-setup.sh
#   cd /tmp/pgit-demo
#   vhs demo.tape

Output demo.gif

Set Shell "bash"
Set FontSize 14
Set Width 1000
Set Height 600
Set Padding 16
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
Sleep 300ms
Type "k"
Sleep 600ms

# back down
Type "j"
Sleep 300ms
Type "j"
Sleep 600ms

# ── Expand commit to see details ────────────────────────

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
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 600ms
# exit diff
Type "q"
Sleep 800ms

# ── Reorder commits ─────────────────────────────────────

# Move to rate limiting
Type "j"
Sleep 300ms
Type "j"
Sleep 500ms

# Move it up twice
Ctrl+k
Sleep 1s
Ctrl+k
Sleep 1.5s

# ── Visual select + squash ──────────────────────────────

# Move to auth commit
Type "k"
Sleep 300ms
Type "k"
Sleep 400ms

# Start selection and extend down
Type "V"
Sleep 500ms
Type "j"
Sleep 500ms

# Squash
Type "s"
Sleep 1.5s

# nano opens — save and exit
Ctrl+o
Sleep 1s
Enter
Sleep 1s
Ctrl+x
Sleep 3s

# ── Rebase onto base ────────────────────────────────────

Type "r"
Sleep 1s
# Confirm
Type "y"
Sleep 4s
Enter
Sleep 1.5s

# ── Help screen ─────────────────────────────────────────

Type "?"
Sleep 3s
Type "q"
Sleep 800ms

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms
