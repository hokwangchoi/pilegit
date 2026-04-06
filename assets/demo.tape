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

# ── Navigate the stack ──────────────────────────────────

Type "k"
Sleep 300ms
Type "k"
Sleep 300ms
Type "k"
Sleep 300ms
Type "k"
Sleep 500ms

# back down
Type "j"
Sleep 300ms
Type "j"
Sleep 500ms

# ── Expand commit to see details ────────────────────────

Enter
Sleep 1.2s
Enter
Sleep 500ms

# ── Diff view ───────────────────────────────────────────

Type "d"
Sleep 1s
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
Sleep 500ms
# exit diff
Type "q"
Sleep 600ms

# ── Visual select + squash ──────────────────────────────

# Move to auth commit
Type "k"
Sleep 300ms
Type "k"
Sleep 300ms

# Start selection and extend down
Type "V"
Sleep 400ms
Type "j"
Sleep 400ms

# Squash
Type "s"
Sleep 1s

# nano opens — save and exit
Ctrl+o
Sleep 500ms
Enter
Sleep 500ms
Ctrl+x
Sleep 1.5s

# ── Undo the squash ─────────────────────────────────────

Type "u"
Sleep 800ms

# ── Reorder commits ─────────────────────────────────────

# Move to rate limiting
Type "j"
Sleep 300ms
Type "j"
Sleep 400ms

# Move it up twice
Ctrl+k
Sleep 700ms
Ctrl+k
Sleep 700ms

# ── Rebase onto base ────────────────────────────────────

Type "r"
Sleep 500ms
# Confirm
Type "y"
Sleep 1.5s
Enter
Sleep 800ms

# ── Undo everything ─────────────────────────────────────

Type "u"
Sleep 500ms
Type "u"
Sleep 500ms
Type "u"
Sleep 800ms

# ── Help screen ─────────────────────────────────────────

Type "?"
Sleep 2.5s
Type "q"
Sleep 500ms

# ── Quit ────────────────────────────────────────────────

Type "q"
Sleep 500ms
