# skit demo — one tape, every language. scripts/record_demo.sh mounts the language's scripts
# at /demo and sets SKIT_LANG, then renames the single /out/demo.mp4 to docs/demo-en.mp4 /
# docs/demo-zh.mp4. Same choreography, matched pacing, two locales — the reason to script it.
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
# below, to show both detection paths. clear — none of this is recorded.
Hide
Type "source ~/.bashrc" Enter
Type "skit add --cmd 'echo {message}' -n my_command" Enter
Type "skit add my_program.sh --exe -n my_program" Enter
Type "clear" Enter
Show

# 1 — A plain script whose parameters are argparse FLAGS, not hard-coded constants.
Type "bat my_script_1.py" Enter
Sleep 3s

# 2 — skit reads the script's OWN command line: three flags become three form fields.
Type "skit add my_script_1.py" Enter
Sleep 1.8s
Enter
Sleep 900ms
Enter
Sleep 2.8s

# 3 — A second script, parameters hard-coded as constants: skit detects them to manage.
Type "skit add my_script_2.py" Enter
Sleep 1.6s
Enter
Sleep 900ms
Enter
Sleep 1.6s
Enter
Sleep 2.6s

# 4 — The library: four scripts of different kinds (○ Python · $ Command · ▶ Program).
Type "skit" Enter
Sleep 3s

# 5 — Run the argparse one. The form is generated from --name / --count / --shout:
#     a text field, a number field, and a checkbox — nothing was hand-written.
Type "/"
Sleep 400ms
Type "script_1"
Sleep 1.2s
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
Tab
Right 30
Backspace 30
Type "Injected!"
Sleep 1.5s
Enter
Sleep 4s
Enter

# 7 — Back in the library; quit.
Sleep 1s
Escape
Sleep 1.5s
