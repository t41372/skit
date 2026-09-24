# skit demo: one tape for every language. scripts/record_demo.sh mounts the language's scripts
# at /demo and sets SKIT_LANG. Then it renames /out/demo.mp4 to docs/assets/demo-{locale}.mp4.
# All locales get the same steps and the same pacing. That is why the demo is scripted.
#
# The output is MP4, not a looping GIF. The demo shows two scripts, and a viewer must be able to
# pause, seek, and replay it.
#
# A person adds each script with `skit add <path>` in the shell. The shell completes the file
# name, so nobody types a directory. In a terminal, this command opens the same review panel as
# `a` in the Library. The two scripts are staggered: add greet, open skit, run greet; add banner,
# open skit, look at it in vim, run it, open its settings. The library holds only Python scripts.
#
# The demo shows what a person does. Do not put `--no-input` or other machine flags on camera.
#
# Pacing: operations are slow on purpose (per-key @time, not instant bursts), so nothing flashes
# past. Where a panel is dense, the tape moves THROUGH it slowly instead of a long Sleep.

Output "/out/demo.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 60ms

# Hidden setup. after_run=exit keeps the output of each short script in the terminal. The steps
# open the Library again after a run. The library starts EMPTY; the tape adds greet and banner
# one at a time.
Hide
Type "source ~/.bashrc" Enter
Type "skit config after_run exit" Enter
Type "clear" Enter
Show

# ── SCRIPT 1 · greet.py ──────────────────────────────────────────────────────
# 1: A plain script. Its parameters are argparse FLAGS, not hard-coded constants. bat shows the
#    source: --name, --count, --shout, --names, and nothing that is specific to skit.
Type "bat greet.py" Enter
Sleep 3.5s
Type "clear" Enter
Sleep 400ms

# 2: `skit add greet.py` opens the review panel in the terminal. The name, the description, and
#    the storage mode are already filled in. Ctrl+S adds the entry.
Type "skit add greet.py" Enter
Sleep 2.5s
Ctrl+S
Sleep 2s

# 3: Open skit. greet is the only entry, and it is selected. Enter opens the run form. skit makes
#    the form from --name, --count, --shout, and --names: a text box, a number box, a checkbox,
#    and a path box. Fill it slowly, run it, and read the output. Then open the Library again,
#    so the completed run shows in the detail pane.
#
#    The path field is the step that a demo must show. --names has the argparse type `Path`, so
#    the field is a path field. The mouse opens the file picker with "📁 browse" on the field row.
#    VHS has no mouse, so this tape uses the keyboard: Ctrl+T opens the insert menu. On a path
#    field, "File or folder…" is the first item, so Enter opens the picker. Type to filter, and
#    Enter takes the highlighted match.
Type "clear" Enter
Type "skit" Enter
Sleep 2.4s
Enter
Sleep 1.8s
Right@20ms 10
Backspace@45ms 10
Type "Ada"
Sleep 800ms
Tab
Sleep 700ms
Right@20ms 6
Backspace@45ms 6
Type "3"
Sleep 800ms
Tab
Sleep 700ms
Space
Sleep 1.1s
Tab
Sleep 900ms
Ctrl+T
Sleep 1.6s
Enter
Sleep 1.8s
Type "names"
Sleep 1.2s
Enter
Sleep 1.6s
Enter
Sleep 3.8s
Type "skit" Enter
Sleep 2s
Type "q"
Sleep 1.3s

# ── SCRIPT 2 · banner.py ─────────────────────────────────────────────────────
# 4: A second script. Its parameters are hard-coded constants (MESSAGE, TIMES, WIDTH). The same
#    `skit add` finds them and ticks each one. Tab moves the focus down to WIDTH, and Space
#    clears its tick: skit must not manage it, so it stays a plain constant.
Type "clear" Enter
Sleep 300ms
Type "skit add banner.py" Enter
Sleep 2.5s
Tab@500ms 7
Sleep 1.2s
Space
Sleep 1.6s
Ctrl+S
Sleep 2s

# 5: Open skit and look inside banner with skit's own editor key: `e` opens $EDITOR, here vim.
#    skit wrote the ticked constants as a PEP 723 [tool.skit] block at the top. The code below
#    it did not change. Move through the file the way a person reads it: j down, then k back up.
#    Do not use Ctrl+D, G, or gg: a jump hides the lines between, and they are the point.
#    Quit vim with :q. This is a view only.
Type "clear" Enter
Type "skit" Enter
Sleep 2.4s
Type "e"
Sleep 1.6s
Type@110ms "jjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjj"
Sleep 1.3s
Type@100ms "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk"
Sleep 1.1s
Type ":q" Enter
Sleep 1.6s

# 6: Run banner. The form is the same, but skit INJECTS the values into a temporary copy. The
#    source file does not change. After the output, open the Library again.
Enter
Sleep 1.8s
Right@20ms 20
Backspace@40ms 20
Type "Injected!"
Sleep 1.1s
Enter
Sleep 3.8s
Type "skit" Enter
Sleep 2.2s

# 7: The constants that skit found are now managed parameters. banner is still selected, so `p`
#    opens ITS settings. The panel is taller than one screen, so move through it slowly from the
#    top to the bottom and back. PageDown and PageUp scroll the panel.
Type "p"
Sleep 2.2s
PageDown
Sleep 1.4s
PageDown
Sleep 1.6s
PageUp
Sleep 1.2s
PageUp
Sleep 1.5s
Escape
Sleep 1.2s

# 8: Back in the Library. Quit.
Type "q"
Sleep 1.5s
