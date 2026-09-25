# skit TUI screenshots: the same container as demo.tape, one tape for every locale.
# scripts/record_demo.sh mounts this tape and the locale's scripts at /demo, sets SKIT_LANG, and
# then renames /out/shot-*.png to docs/assets/tui-*-{locale}.png. There are four screens, one for
# each image in the README grid:
#
#   shot-add.png       the add review panel, with the constants that skit found in banner
#   shot-library.png   the Library (list, detail pane, and footer)
#   shot-settings.png  the Entry settings of banner, just after the add
#   shot-form.png      the run form that skit made from the argparse flags of greet
#
# The library holds only Python scripts. The hidden setup adds greet with `--no-input`, which
# is off camera. The tape adds banner LIVE in shot 1: that shows the review panel, and Ctrl+S
# then manages the constants that the settings shot needs. `skit add x.py --no-input` does not
# manage a constant.
#
# VHS needs an mp4 Output; record_demo.sh deletes it. VHS takes a screenshot about one command
# late, so each Screenshot gets a Sleep after it. Without it, the image shows the next step.

Output "/out/shots.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 40ms

Hide
Type "source ~/.bashrc" Enter
Type "skit add greet.py --no-input" Enter
Sleep 1.5s
Type "clear" Enter
Type "skit" Enter
Sleep 3s
Show

# 1: The add review panel. `a`, then banner.py, then Enter opens the panel. skit found MESSAGE,
#    TIMES, and WIDTH and ticked each one. Five Tabs put the focus on the first tick box, so the
#    footer shows Space Toggle. Ctrl+S adds banner, and that also fills the Library for shot 2.
Type "a"
Sleep 1.5s
Type "banner.py"
Sleep 800ms
Enter
Sleep 2.5s
Tab 5
Sleep 1s
Screenshot "/out/shot-add.png"
Sleep 2s
Ctrl+S
Sleep 2s

# 2: The Library with two Python scripts (banner and greet), the detail pane, and the footer.
Screenshot "/out/shot-library.png"
Sleep 2s

# 3: Entry settings. banner is selected, so `p` opens ITS settings. The constants that skit found
#    are now managed parameters.
Type "p"
Sleep 2.5s
Screenshot "/out/shot-settings.png"
Sleep 2s
Escape
Sleep 1s

# 4: The run form of greet (a text box, a number box, a check box, and a path box), filled in.
#    Search for greet, open it, and fill the fields. The path comes from the file picker, not from
#    typing: Ctrl+T opens the insert menu, "File or folder…" is the first item on a path field,
#    Enter opens the picker, and the filter takes the first match.
Type "/"
Sleep 400ms
Type "greet"
Sleep 1s
Enter
Sleep 2.5s
Right 20
Backspace 20
Type "Ada"
Tab
Right 20
Backspace 20
Type "3"
Tab
Space
Tab
Sleep 600ms
Ctrl+T
Sleep 1.2s
Enter
Sleep 1.4s
Type "names"
Sleep 1s
Enter
Sleep 1.5s
Screenshot "/out/shot-form.png"
Sleep 2s
Escape
Sleep 1.5s
