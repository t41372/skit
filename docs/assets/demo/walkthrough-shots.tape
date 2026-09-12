# walkthrough-shots.tape — one screenshot per distinct TUI state.
#
# Companion to walkthrough.tape. That tape tells the story; this one is the evidence.
#
# Three rules keep the frames comparable:
#   1. Terminal size is fixed at 1280x860 = 101x31 cells. A fixed size is what makes two
#      recordings comparable at all. 31 rows also leave the entry settings body only a
#      little taller than its viewport, so the w11 frame catches a control clipped to two
#      rows that still shows its value. settings-clip.tape records the same screens at
#      101x28, where the body scrolls further and that control keeps only its last row.
#   2. Set CursorBlink false is declared so cursor phase cannot vary with wall-clock time.
#      vhs validate accepts the setting, but VHS also accepts settings it then ignores, so
#      treat this as unproven at runtime. The byte-stable w03 frame, which shows a cursor
#      block, is indirect evidence that it does take effect.
#   3. Every Wait pattern is locale invariant. It matches an entry name, a code constant,
#      or a chord label, never translated prose. The zh-CN and zh-TW runs use this same file.
#
# A Wait that fails prints the whole screen and stops the tape. That is deliberate: a beat
# that lands in the wrong place must fail loudly, not drift on into the next scene.
#
# Not every frame is byte reproducible between two runs. Some frames raster one beat late
# while the emulator text grid is already correct. That is a VHS capture artifact, not a
# skit defect. record_walkthrough.sh records this tape twice and prints, frame by frame,
# which pairs matched and which did not. Read the text on a frame, not its bytes.

Output "/out/walkthrough-shots.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 860
Set Padding 20
Set TypingSpeed 40ms
Set CursorBlink false
Set WaitTimeout 30s

# Seed the library from the command line. Hidden: the shell is not the subject.
Hide
Type "source ~/.bashrc" Enter
Sleep 1s
Type "skit add greet.py --no-input" Enter
Sleep 3s
Type "skit add banner.py --no-input" Enter
Sleep 3s
Type "clear" Enter
Sleep 1s
Show

# ---------------------------------------------------------------- library

Type "skit" Enter
Wait+Screen /banner/
Sleep 2s
# The library sorts by activity descending, then slug ascending, so the row order is
# deterministic. Up clamps at the first row. That pins the selection to a known entry
# whatever the initial selection rule picks, and keeps every later beat stable.
# VHS has no Home key, so Up with a repeat count is the way to do it.
Up 4
Sleep 2s
Screenshot "/out/w01-library.png"

# Move to the second row. The detail pane follows the selection.
Down
Sleep 2s
Screenshot "/out/w02-library-detail.png"

# ---------------------------------------------------------------- search

Type "/"
Sleep 1s
Type "gre"
Wait+Screen /greet/
Sleep 2s
Screenshot "/out/w03-search.png"

# Ctrl+U clears the query. Esc alone leaves the search box but keeps the filter, so the
# list would stay narrowed to one row.
Ctrl+U
Sleep 1s
Escape
Wait+Screen /banner/
Sleep 2s

# Tab hides the detail pane and pins it hidden.
Tab
Sleep 2s
Screenshot "/out/w04-detail-hidden.png"
Tab
Sleep 2s

# ---------------------------------------------------------------- run form

Enter
Wait+Screen /World/
Sleep 2s
Screenshot "/out/w05-run-form.png"

# Ctrl+T offers the run-time values a field can take.
Ctrl+T
Wait+Screen /cwd/
Sleep 2s
Screenshot "/out/w06-token-menu.png"
Escape
Sleep 2s

# Ctrl+S saves the filled form as a preset.
Ctrl+S
Sleep 2s
Screenshot "/out/w07-preset-modal.png"
Type "demo"
Wait+Screen /demo/
Sleep 2s
Screenshot "/out/w08-preset-named.png"
Enter
Sleep 3s
Screenshot "/out/w09-preset-saved.png"

Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- entry settings

# p opens entry settings at the top.
Type "p"
Wait+Screen /greet.py/
Sleep 2s
Screenshot "/out/w10-entry-settings.png"

Escape
Wait+Screen /banner/
Sleep 2s

# s opens the same screen deep-linked to the Presets section.
Type "s"
Wait+Screen /demo/
Sleep 2s
Screenshot "/out/w11-presets-deeplink.png"

Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- preferences, health, agents

Type ","
Wait+Screen /VISUAL/
Sleep 2s
Screenshot "/out/w12-preferences.png"

Escape
Wait+Screen /banner/
Sleep 2s

Type "D"
Wait+Screen /uv/
Sleep 2s
Screenshot "/out/w13-health.png"

Escape
Wait+Screen /banner/
Sleep 2s

Type "R"
Wait+Screen /codex/
Sleep 2s
Screenshot "/out/w14-agents.png"

Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- add flow

Type "a"
Wait+Screen /Ctrl.O/
Sleep 2s
Screenshot "/out/w15-add-source.png"

# pep723.py carries its own PEP 723 block with an empty dependency list. The review
# stage reads that block instead of guessing, and says so.
Type "pep723.py"
Sleep 1s
Enter
Wait+Screen /PEP 723/
Sleep 2s
Screenshot "/out/w16-add-review-pep723.png"

Ctrl+S
Wait+Screen /pep723/
Sleep 3s
Screenshot "/out/w17-library-three.png"

# ---------------------------------------------------------------- remove confirmation

Delete
Sleep 2s
Screenshot "/out/w18-confirm-remove.png"
Escape
Wait+Screen /banner/
Sleep 2s

# ---------------------------------------------------------------- help overlay

# The footer advertises "? Help", and ? opens it. The overlay lists every library command
# beside its advertised chord. Chords are not translated, so the doubled quit chord is a
# locale-invariant anchor, and it appears nowhere else on the library screen.
Type "?"
Wait+Screen /Ctrl.C Ctrl.C/
Sleep 2s
Screenshot "/out/w19-help-overlay.png"

# Esc closes the overlay, as the overlay's own footer says. q does nothing while Help is
# open, so the tape must close the overlay before it quits.
Escape
Wait+Screen /banner/
Sleep 2s
Screenshot "/out/w20-help-closed.png"

Type "q"
Sleep 2s
