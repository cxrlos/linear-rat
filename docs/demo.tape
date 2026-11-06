# Records docs/demo.gif with VHS (https://github.com/charmbracelet/vhs).
#
# lrat --demo runs on the built-in fake workspace, so this needs no Linear
# account and no network. It runs target/release/lrat and does not build it,
# so build first, from the repository root:
#
#   cargo build --release --locked
#   vhs docs/demo.tape
#
# lrat's config dir points at a directory that does not exist, so the
# recording uses the default config (and the terminal theme below) whatever
# my own config.toml says. It is set on the lrat command, not with Env: Env
# also reaches the browser VHS records with, which then fails to start.
# My Issues shows the detail beside the list from 120
# columns; the size below is meant to clear that, but the column count is an
# estimate, so check the first frames show the detail on the right.

Output docs/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1400
Set Height 800
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 120ms

Hide
Type "clear && XDG_CONFIG_HOME=/nonexistent/lrat-demo target/release/lrat --demo"
Enter
Sleep 1.5s
Show

# My Issues with the detail beside the list.
Sleep 2s
Type "jjj"
Sleep 1s
Type "l"
Sleep 800ms
Type "jj"
Sleep 800ms
Type "h"
Sleep 1s

# Change the workflow state through the picker: filter, then choose.
Type "s"
Sleep 1s
Type "rev"
Sleep 800ms
Enter
Sleep 1.5s

# The palette finds actions and loaded issues.
Type ":"
Sleep 600ms
Type "cheese"
Sleep 1s
Enter
Sleep 1.5s
Escape
Sleep 800ms

# Team Issues, filtered by workflow state with a number key.
Type "2"
Sleep 1.5s
Type "fs"
Sleep 1s
Type "2"
Sleep 1.5s

# Analytics: a longer range, then the Team view.
Type "3"
Sleep 2s
Type "]"
Sleep 1.5s
Type "v"
Sleep 2.5s

# Every key of the current screen.
Type "1"
Sleep 600ms
Type "?"
Sleep 2s
Escape
Sleep 600ms

Type "q"
Sleep 500ms
