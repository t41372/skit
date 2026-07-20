# skit TUI screenshots — same hermetic pipeline as demo.tape, one tape for every locale.
# scripts/record_demo.sh mounts this tape and the locale's scripts at /demo, sets SKIT_LANG,
# then renames /out/shot-*.png to docs/assets/tui-*-{en,zh}.png. Four key screens, matching
# the README's four-image grid:
#
#   shot-library.png   — the library workbench (list + details + two-line footer)
#   shot-form.png      — the run form, generated from greet's argparse flags
#   shot-add.png       — the add review panel, constants detected in banner
#   shot-settings.png  — the script-settings screen of the freshly added banner
#
# The library holds ONLY Python scripts (skit's exe / command-template support is still thin,
# so the demo never shows those kinds). greet is pre-added silently; banner is added LIVE in
# shot 1, which both captures the add panel AND populates the library for shot 2 — and a live
# Ctrl+S is what gives banner the managed constants the settings shot needs (an interactive
# `skit add x.py --no-input` deliberately manages nothing).
#
# The mp4 Output is mandatory for VHS; record_demo.sh deletes it.

Output "/out/shots.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 40ms

# Hidden setup: pre-add greet (the argparse script) silently so the library isn't empty.
# banner stays UN-added — shot 1 adds it live.
Hide
Type "source ~/.bashrc" Enter
Type "skit add greet.py --no-input" Enter
Sleep 1.5s
Type "clear" Enter
Type "skit" Enter
Sleep 3s
Show

# 1 — Add review panel: `a` → banner.py → the panel opens with the constants (Message / Times
#     / Width) detected and pre-ticked. Tab down so the checklist is in view, screenshot, then
#     Ctrl+S commits banner (managed constants and all) — which also fills the library for shot 2.
Type "a"
Sleep 1.5s
Type "banner.py"
Sleep 800ms
Enter
Sleep 2.5s
Tab 4
Sleep 1s
Screenshot "/out/shot-add.png"
# VHS captures screenshots with a lag of roughly the next command, so every Screenshot gets a
# settle Sleep — otherwise the frame shows the NEXT interaction (this shipped a library shot
# with the search-focused footer once).
Sleep 2s
Ctrl+S
Sleep 2s

# 2 — Library workbench: two Python scripts (banner · greet), list + detail + two-line footer.
Screenshot "/out/shot-library.png"
Sleep 2s

# 3 — Script settings: banner is selected (fresh add), so `p` opens ITS settings — the
#     constants it detected are now managed params.
Type "p"
Sleep 2.5s
Screenshot "/out/shot-settings.png"
Sleep 2s
Escape
Sleep 1s

# 4 — Run form for the argparse script (text / number / checkbox), partly filled in. Search
#     greet, run it, fill a couple of fields.
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
Sleep 1.2s
Screenshot "/out/shot-form.png"
Sleep 2s
Escape
Sleep 1.5s
