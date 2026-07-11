# skit TUI screenshots — same hermetic pipeline as demo.tape, one tape for every locale.
# scripts/record_demo.sh mounts this tape and the locale's scripts at /demo, sets SKIT_LANG,
# then renames /out/shot-*.png to docs/assets/tui-*-{en,zh}.png. Four key screens, matching
# the README's four-image grid:
#
#   shot-library.png   — the library workbench (list + details + two-line footer)
#   shot-form.png      — the run form, generated from my_script_1's argparse flags
#   shot-add.png       — the add review panel, constants detected in my_script_2
#   shot-settings.png  — the script-settings screen of the freshly added my_script_2
#
# The mp4 Output is mandatory for VHS; record_demo.sh deletes it.

Output "/out/shots.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1200
Set Height 780
Set Padding 20
Set TypingSpeed 40ms

# Hidden setup: three entries pre-added for a lively library (command template, executable,
# the argparse script). my_script_2.py stays UN-added — the add-panel shot adds it live.
Hide
Type "source ~/.bashrc" Enter
Type "skit add --cmd 'echo {message}' -n my_command" Enter
Type "skit add my_program.sh --exe -n my_program" Enter
Sleep 500ms
Type "skit add my_script_1.py" Enter
Sleep 1.5s
Enter
Sleep 800ms
Enter
Sleep 1.5s
Type "clear" Enter
Type "skit" Enter
Sleep 3s
Show

# 1 — Library workbench.
Screenshot "/out/shot-library.png"

# 2 — Run form for the argparse script (text / number / checkbox), partly filled in.
Type "/"
Sleep 400ms
Type "script_1"
Sleep 1s
Enter
Sleep 2.5s
Tab
Right 20
Backspace 20
Type "Ada"
Tab
Right 20
Backspace 20
Type "3"
Tab
Space
Sleep 1.2s
Screenshot "/out/shot-form.png"
Escape
Sleep 1.5s

# Clear the leftover search filter (text persists across the run round-trip).
Type "/"
Sleep 300ms
Backspace 20
Escape
Sleep 500ms

# 3 — Add review panel. Tab down so the detected-constants checklist scrolls into view
#     (the panel opens at the top; the differentiator lives below the fold).
Type "a"
Sleep 1.5s
Type "my_script_2.py"
Sleep 800ms
Enter
Sleep 2.5s
Tab 5
Sleep 1s
Screenshot "/out/shot-add.png"

# 4 — Add it (Ctrl+A, per the footer), then open script settings on the new entry.
Ctrl+A
Sleep 2s
Type "p"
Sleep 2.5s
Screenshot "/out/shot-settings.png"

Escape
Sleep 800ms
Escape
Sleep 1s
