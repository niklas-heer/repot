# The README demo. Render it with `mise run demo`, which builds repot first.
# demo/setup.sh creates an isolated home with offline remotes; see its header.

Output demo/repot.gif

Require bash
Require git

Set Shell bash
Set FontSize 18
Set Width 1400
Set Height 800
Set Padding 28
Set Margin 36
Set MarginFill "#D9623B"
Set WindowBar Colorful
Set WindowBarSize 36
Set BorderRadius 12
Set TypingSpeed 55ms
Set CursorBlink false
# Potting soil, terracotta and leaf green, taken from the repot logo.
Set Theme { "name": "repot", "background": "#1E1916", "foreground": "#EFE6DA", "cursor": "#E8774B", "selection": "#3A302A", "black": "#2A231F", "red": "#EF6F5A", "green": "#8CC96B", "yellow": "#F4C569", "blue": "#7EAEEA", "magenta": "#E28FCF", "cyan": "#6FD0C2", "white": "#EFE6DA", "brightBlack": "#8C7F74", "brightRed": "#FF8A73", "brightGreen": "#A6DE85", "brightYellow": "#FFD88A", "brightBlue": "#9CC3F5", "brightMagenta": "#F0A9E0", "brightCyan": "#8FE3D6", "brightWhite": "#FFFFFF" }

Hide
Type 'export PATH="$PWD/target/release:$PATH" && sh demo/setup.sh >/dev/null && source /tmp/repot-demo/home/.bashrc && clear'
Enter
Sleep 4s
Show

Sleep 500ms

# One fetch, one grouped report of what needs attention.
Type "repot status"
Sleep 400ms
Enter
Sleep 4.5s

# Fast-forward what is safe; the merged branch returns to main.
Type "repot sync"
Sleep 400ms
Enter
Sleep 4.5s

Hide
Type "clear"
Enter
Show

# Jump anywhere with the inline fuzzy picker, ranked by use.
Type "repot cd"
Sleep 400ms
Enter
Sleep 2s
Down
Sleep 700ms
Down
Sleep 700ms
Up@350ms 2
Sleep 500ms
Type@150ms "desi"
Sleep 1.5s
Ctrl+U
Sleep 400ms
Type@150ms "api"
Sleep 1.5s
Enter
Sleep 1s

Type "repot info"
Sleep 400ms
Enter
Sleep 4s

Hide
Type "clear"
Enter
Show

# Clone into the tree, then jump straight to a unique match.
Type "repot clone octo/dotfiles"
Sleep 400ms
Enter
Sleep 2.5s

Type "repot cd dotfiles"
Sleep 400ms
Enter
Sleep 3s
