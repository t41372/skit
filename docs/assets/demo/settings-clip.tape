# settings-clip.tape — regression evidence for the clipped-control fix.
#
# Entry settings lays every text control out at three rows and draws the field name on the
# box's top border row. At 101x28 the body is taller than the viewport, so the Presets deep
# link scrolls it and the Name control ends up only part way inside the viewport.
#
# skit used to hand such a control its CLIPPED rectangle and let it draw itself. The control
# then drew its FIRST row into whatever space was left, wherever on screen that space was.
# Scrolled down to one surviving row, the screen showed the control's top border, carrying
# the field name, sitting where its bottom border belongs, and the value row was gone.
#
# skit now paints only the visible BAND of the control, so a surviving row is the row that
# actually belongs at that position.
#
# The two frames below are the evidence, and they do not show the same thing:
#   clip-02, opened at the top, shows the whole control: the title "Name" and the value
#   "greet" between its two borders. That is the control frame.
#   clip-03, opened through the scrolled deep link, keeps only the control's LAST row inside
#   the viewport. It must show a plain border rule directly above "Renaming keeps
#   everything…" and NO field name. A "Name" title on that line is the defect returning: it
#   would mean the control had drawn its first row in its last row's place.
#
# The other half of the fix, keeping the value on a part-visible control, is easier to see at
# 101x31, where two rows of the control survive: the w11 frame of walkthrough-shots.tape
# shows "greet" with the title clipped away. That is why both sizes are recorded.
#
# This file replaces an earlier tape that recorded the same drive while the defect was live.
# Keep the size at 1280x780 = 101x28: a taller terminal does not scroll the body this far and
# would not reach the one-surviving-row case.

Output "/out/settings-clip.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 40ms
Set CursorBlink false
Set WaitTimeout 30s

Hide
Type "source ~/.bashrc" Enter
Sleep 1s
Type "skit add greet.py --no-input" Enter
Sleep 3s
Type "clear" Enter
Sleep 1s
Show

Type "skit" Enter
Wait+Screen /greet/
Sleep 2s
Screenshot "/out/clip-01-library.png"

# p opens entry settings at the TOP, where the Name control fits whole. This is the control
# frame: it shows what the field looks like when nothing is clipped.
Type "p"
Wait+Screen /greet.py/
Sleep 3s
Screenshot "/out/clip-02-settings-top.png"
Escape
Wait+Screen /greet/
Sleep 2s

# s opens the SAME screen deep-linked to Presets, which scrolls the body down until only the
# Name control's closing border is still inside the viewport. That border, with no field name
# on it, is what this frame must show.
Type "s"
Wait+Screen /greet.py/
Sleep 3s
Screenshot "/out/clip-03-settings-scrolled.png"

# Leave entry settings before quitting, and never follow a Screenshot with a character key
# while a text field has focus. This screen opens with Name focused, so a typed "q" goes into
# the field instead of quitting: it renames the entry to "greetq" and scrolls the body back
# to the top to show the field it just changed. A screenshot that rasters late then records
# that mutated screen as if it were this beat. Esc is safe from either state.
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s
Type "q"
Sleep 2s
