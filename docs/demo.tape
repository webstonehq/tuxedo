# VHS script for the README hero GIF.
#
#   vhs docs/demo.tape  →  docs/demo.gif
#
# Four scenes on an empty todo.txt:
#   1. Empty state
#   2. Natural-language add
#   3. Slash-command metadata picker
#   4. Command palette to toggle complete

Output docs/demo.gif

Set FontSize 14
Set Width 1200
Set Height 720
Set Padding 12
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms
Set Shell "bash"

# Prep a clean empty file and an isolated XDG_CONFIG_HOME so the demo always
# starts from default prefs, regardless of the user's local config. Hidden so
# the recording opens directly into the TUI.
Hide
Type "rm -rf /tmp/tuxedo-demo-cfg && mkdir -p /tmp/tuxedo-demo-cfg && rm -f /tmp/tuxedo-demo.txt /tmp/tuxedo-demo.done.txt && touch /tmp/tuxedo-demo.txt && export TUXEDO_NO_UPDATE_CHECK=1 XDG_CONFIG_HOME=/tmp/tuxedo-demo-cfg && clear"
Enter
Show

Type "./target/release/tuxedo /tmp/tuxedo-demo.txt"
Enter

# 1. Empty state — bowtie logo + quickstart panel.
Sleep 2500ms

# 2. Natural-language add — prose rewrites to canonical todo.txt on the first
# Enter, the second Enter saves.
Type "n"
Sleep 700ms
Type "Pay rent monthly on the first, project home"
Sleep 1200ms
Enter
Sleep 2000ms
Enter
Sleep 1500ms

# 3. Slash-command metadata — type a body, then `/due` opens a calendar and
# `/prio` opens the priority chooser. Enter accepts each step.
Type "n"
Sleep 700ms
Type "Buy birthday gift"
Sleep 800ms
Type " /due"
Sleep 1000ms
Enter
Sleep 1000ms
# Calendar: `w` jumps forward one week, Enter saves.
Type "w"
Sleep 1000ms
Enter
Sleep 1200ms
Type " /prio"
Sleep 1000ms
Enter
Sleep 1000ms
# Priority chooser: A is preselected. Enter accepts, second Enter saves the task.
Enter
Sleep 1200ms
Enter
Sleep 1800ms

# 4. Command palette — `:` opens a fuzzy palette over every action. Cursor is
# still on the just-added birthday gift; "toggle" filters to "toggle complete"
# and Enter marks the row done.
Type ":"
Sleep 1000ms
Type "toggle"
Sleep 1500ms
Enter
Sleep 2000ms
