# skit demo — one tape, every language. scripts/record_demo.sh mounts the language's scripts
# at /demo and sets SKIT_LANG, then renames the single /out/demo.mp4 to docs/assets/demo-en.mp4 /
# docs/assets/demo-zh.mp4. Same choreography, matched pacing, two locales — the reason to script it.
#
# Output is MP4 (pausable / seekable / replayable) rather than a looping GIF: this demo packs
# two capabilities and rewards a pause.
#
# First-render tuning: eyeball the video once and adjust Sleep values / the Tab-focus counts
# in the run scenes below. That short loop is normal VHS workflow.

Output "/out/demo.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1200
Set Height 780
Set Padding 20
Set TypingSpeed 55ms

# Hidden setup: recording shell + two filler library entries (a command template and an
# executable) so the library shows variety of kinds. The two STAR scripts are added live
# below, to show both detection paths. after_run=stay because this demo tells the
# WORKBENCH story (run, land back in the library, rerun) — the out-of-box default is
# "exit": run a script, skit quits like a launcher. clear — none of this is recorded.
Hide
Type "source ~/.bashrc" Enter
Type "skit add --cmd 'echo {message}' -n my_command" Enter
Type "skit add my_program.sh --exe -n my_program" Enter
Type "skit config after_run stay" Enter
Type "clear" Enter
Show

# 1 — A plain script whose parameters are argparse FLAGS, not hard-coded constants.
Type "bat my_script_1.py" Enter
Sleep 3s
# Its flags have landed; wipe the listing so the two `skit add` flows below present on a
# clean prompt instead of stacking under 20 lines of source.
Type "clear" Enter

# 2 — skit reads the script's OWN command line: `skit add` opens the review panel
#     right in the terminal — name, description and detection prefilled; Ctrl+A accepts.
Type "skit add my_script_1.py" Enter
Sleep 3.2s
Ctrl+A
Sleep 2.8s

# 3 — A second script, parameters hard-coded as constants: the same panel shows the
#     constants skit detected, pre-ticked for the run form to manage.
Type "skit add my_script_2.py" Enter
Sleep 3.2s
Ctrl+A
Sleep 2s
Enter
Sleep 2.6s

# 4 — The library: four scripts of different kinds (○ Python · $ Command · ▶ Program).
#     Clear the add-flow chatter first: skit runs scripts by suspending back to THIS
#     terminal (tui.py suspend()), so a clean slate here is what keeps every run below —
#     and the final quit — from surfacing the piled-up `bat` + `skit add` scrollback.
Type "clear" Enter
Type "skit" Enter
Sleep 3s

# 5 — Run the argparse one. The form is generated from --name / --count / --shout
#     and opens with the FIRST FIELD focused (no leading Tab needed):
#     a text field, a number field, and a checkbox — nothing was hand-written.
Type "/"
Sleep 400ms
Type "script_1"
Sleep 1.2s
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
Sleep 1.5s
Enter
Sleep 3.5s
Enter

# 6 — Run the constants one. Same form UI, but the values are INJECTED into a temporary
#     copy — the run output says so, and your source file is never edited.
Sleep 1s
Type "/"
Sleep 400ms
Backspace 20
Type "script_2"
Sleep 1.2s
Enter
Sleep 2.5s
Right 30
Backspace 30
Type "Injected!"
Sleep 1.5s
Enter
Sleep 4s
Enter

# 7 — The constants skit detected became managed settings. my_script_2 is still filtered
#     and selected (from scene 6), so `p` opens ITS settings. The panel runs past one
#     screen — Basics · Storage · managed params · presets · deps — so scroll it top-to-
#     bottom and back. On open the VerticalScroll body holds focus, so PageUp/PageDown
#     scroll the panel itself (Input never eats those keys); ~one page per press.
Sleep 1s
Type "p"
Sleep 2s
PageDown
Sleep 1s
PageDown
Sleep 1.6s
PageUp
Sleep 1s
PageUp
Sleep 1.6s
Escape
Sleep 1s

# 8 — Back in the library; quit.
Escape
Sleep 1.5s
