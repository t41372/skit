# walkthrough.tape — the whole reachable TUI, in one continuous take.
#
# The repo's demo.tape shows four beats. This tape walks every screen a keyboard can reach:
# library, search, detail pane, the help overlay, the run form, the run-time value menu, the
# preset modal, entry settings, the presets deep link, preferences, health, agents, the add
# flow through its review stage, the remove confirmation, and a real run.
#
# Pacing rule, learned from demo.tape's beat drift: never move to the next scene on a Sleep
# alone. Each transition waits for text that only the target screen shows. A wait that fails
# prints the screen and stops the tape, so a keystroke can never land in the wrong field and
# be recorded as if it worked.
#
# Wait patterns are locale invariant on purpose - an entry name, a code constant, or a chord
# label, never translated prose. The same file records en, zh-CN and zh-TW.
#
# Size is 1280x860 = 101x31, which gives entry settings room to draw its fields without
# scrolling. settings-clip.tape records the same screens at 101x28, where the body scrolls.

Output "/out/walkthrough.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 860
Set Padding 20
Set TypingSpeed 55ms
Set CursorBlink false
Set WaitTimeout 30s

Hide
Type "source ~/.bashrc" Enter
Type "clear" Enter
Show

# ---------------------------------------------------------------- 1. the script

Type "bat greet.py" Enter
Sleep 4s
Type "clear" Enter
Sleep 1s

# ---------------------------------------------------------------- 2. add from the shell

Type "skit add greet.py --no-input" Enter
Sleep 3s
Type "skit add banner.py --no-input" Enter
Sleep 3s
Type "clear" Enter
Sleep 1s

# ---------------------------------------------------------------- 3. the library

Type "skit" Enter
Wait+Screen /banner/
Sleep 3s

# The library sorts by activity descending, then slug ascending, so the row order is
# deterministic. Up clamps at the first row. That pins the selection to a known entry
# whatever the initial selection rule picks, and keeps every later beat stable.
# VHS has no Home key, so Up with a repeat count is the way to do it.
Up 4
Sleep 2s


# The detail pane follows the selection.
Down
Sleep 3s

# ---------------------------------------------------------------- 4. help

# The footer advertises "? Help". Pressing ? opens the overlay, which lists every library
# command beside its advertised chord. Esc closes it; q does nothing while it is open.
Type "?"
Wait+Screen /Ctrl.C Ctrl.C/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- 5. search

Type "/"
Sleep 1s
Type "gre"
Wait+Screen /greet/
Sleep 3s
# Ctrl+U clears the query. Esc alone keeps the filter.
Ctrl+U
Sleep 1s
Escape
Wait+Screen /banner/
Sleep 2s

# Tab hides the detail pane, and pins it hidden.
Tab
Sleep 2s
Tab
Sleep 2s

# ---------------------------------------------------------------- 6. the run form

Enter
Wait+Screen /World/
Sleep 3s

# Ctrl+T lists the values skit can fill in at run time.
Ctrl+T
Wait+Screen /cwd/
Sleep 3s
Escape
Sleep 2s

# Ctrl+S saves the filled form as a preset.
Ctrl+S
Sleep 2s
Type "demo"
Wait+Screen /demo/
Sleep 2s
Enter
Sleep 3s

Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- 7. entry settings

Type "p"
Wait+Screen /greet.py/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

# s opens the same screen deep-linked to Presets, so the new preset is on screen.
Type "s"
Wait+Screen /demo/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- 8. preferences, health, agents

Type ","
Wait+Screen /VISUAL/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

Type "D"
Wait+Screen /uv/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

Type "R"
Wait+Screen /codex/
Sleep 4s
Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- 9. the add flow

Type "a"
Wait+Screen /Ctrl.O/
Sleep 3s

# pep723.py carries its own PEP 723 block with an empty dependency list. The review stage
# reads that block instead of guessing from the imports, and says so in as many words.
Type "pep723.py"
Sleep 1s
Enter
Wait+Screen /PEP 723/
Sleep 4s

Ctrl+S
Wait+Screen /pep723/
Sleep 3s

# ---------------------------------------------------------------- 10. remove, then keep

Delete
Sleep 3s
Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- 11. run it

# Preferences keeps "Quit skit after a run", so the run ends the session and leaves its
# output in the terminal. That is the finale.
# Land on greet from a known row rather than from whatever the add left selected.
Up 4
Sleep 1s
Down 2
Sleep 2s
Enter
Wait+Screen /World/
Sleep 2s
Enter
Sleep 6s
