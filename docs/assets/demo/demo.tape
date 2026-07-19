# skit demo — one tape, every language. scripts/record_demo.sh mounts the language's scripts
# at /demo and sets SKIT_LANG, then renames the single /out/demo.mp4 to docs/assets/demo-en.mp4 /
# docs/assets/demo-zh.mp4. Same choreography, matched pacing, two locales — the reason to script it.
#
# Output is MP4 (pausable / seekable / replayable) rather than a looping GIF: this demo packs
# two capabilities and rewards a pause.
#
# Scripts are added with `skit add <path>` FROM THE SHELL — the common path, since the shell
# tab-completes the file so nobody hand-types a directory. Interactively that opens skit's TUI
# review panel (the same one the library's `a` pushes). The two stars are staggered: add greet →
# launch skit → run greet; add banner → launch skit → peek at it in vim → run it → its settings.
# The library holds ONLY Python scripts — skit's exe / command-template support is still thin.
#
# Pacing: operations are deliberately un-hurried (per-key @time, not instant bursts) so nothing
# flashes past; where a panel is dense we move THROUGH it slowly rather than freezing on a Sleep.
# First-render tuning: eyeball once and nudge the @time / Sleep values. That loop is normal VHS.

Output "/out/demo.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 60ms

# Hidden setup: use the default after_run=exit behavior so each short script's output remains
# readable in the terminal. The choreography explicitly relaunches the Library afterward instead
# of relying on stay mode's immediate redraw. The library starts EMPTY; greet and banner are added
# with `skit add` below, one at a time. clear — none of this is recorded.
Hide
Type "source ~/.bashrc" Enter
Type "skit config after_run exit" Enter
Type "clear" Enter
Show

# ── SCRIPT 1 · greet.py ──────────────────────────────────────────────────────
# 1 — A plain script whose parameters are argparse FLAGS, not hard-coded constants. bat shows
#     the source: three flags (--name / --count / --shout), nothing skit-specific.
Type "bat greet.py" Enter
Sleep 3.5s
Type "clear" Enter
Sleep 400ms

# 2 — `skit add greet.py` at the shell opens skit's review panel right in the terminal — name,
#     description and detection prefilled. Walk slowly DOWN the panel (Tab/↓) and back UP
#     (Shift+Tab/↑) — the footer advertises both — so the whole thing is read, then Ctrl+S adds it.
Type "skit add greet.py" Enter
Sleep 1.8s
Tab@550ms 3
Sleep 1s
Shift+Tab
Sleep 450ms
Shift+Tab
Sleep 450ms
Shift+Tab
Sleep 1s
Ctrl+S
Sleep 2s

# 3 — Launch skit; greet is the only entry, already selected. Run it: the form is generated from
#     --name / --count / --shout and opens on the FIRST FIELD — a text box, a number box, a
#     checkbox, nothing hand-written. Fill it unhurriedly, run, read the retained output, then
#     relaunch the Library once so the completed run is visible before moving on.
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
Enter
Sleep 3.5s
Type "skit" Enter
Sleep 2s
Escape
Sleep 1.3s

# ── SCRIPT 2 · banner.py ─────────────────────────────────────────────────────
# 4 — A second script, parameters hard-coded as constants (MESSAGE / TIMES / WIDTH). Same
#     `skit add`; walk down onto the detected-constants checklist, UNTICK Width (skit shouldn't
#     manage it — it stays a plain constant), walk back up, then add.
Type "clear" Enter
Sleep 300ms
Type "skit add banner.py" Enter
Sleep 1.8s
Tab@550ms 4
Sleep 1.1s
Tab@550ms 3
Sleep 900ms
Space
Sleep 1.3s
Shift+Tab
Sleep 450ms
Shift+Tab
Sleep 450ms
Shift+Tab
Sleep 450ms
Shift+Tab
Sleep 1s
Ctrl+S
Sleep 2s

# 5 — Launch skit; banner is selected (freshest add). Peek inside with skit's OWN editor: `e`
#     opens $EDITOR — here vim. skit recorded the ticked constants as a PEP 723 [tool.skit] block
#     at the top and left the code below untouched. Page briskly top-to-bottom so the whole
#     (short) script is seen, then quit vim (:q) — view only.
Type "clear" Enter
Type "skit" Enter
Sleep 2.4s
Type "e"
Sleep 1.6s
Ctrl+D
Sleep 1.5s
Type "G"
Sleep 1.3s
Type "gg"
Sleep 1s
Type ":q" Enter
Sleep 1.6s

# 6 — Run banner. Same form UI, but the values are INJECTED into a temporary copy — the run
#     output says so, and your source file is never edited. Relaunch the Library after the retained
#     output, then open the selected entry's settings.
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

# 7 — The constants skit detected became managed settings. banner is still selected, so `p`
#     opens ITS settings. The panel runs past one screen — Basics · Storage · managed params ·
#     presets · deps — so scroll it slowly top-to-bottom and back (PageUp/PageDown move the body).
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

# 8 — Back in the library; quit.
Escape
Sleep 1.5s
