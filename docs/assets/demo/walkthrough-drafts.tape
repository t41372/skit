# walkthrough-drafts.tape — the kept-draft path, kept apart on purpose.
#
# "Write a script…" names its draft skit-new-XXXXXX.py with six random characters. That name
# is on screen in the confirmation, so these frames CANNOT be byte reproducible. They are
# excluded from the determinism gate. Everything reproducible lives in walkthrough-shots.tape.
#
# The route to the confirmation is not obvious, and the frames below prove each step:
#   Ctrl+N writes a draft and opens $EDITOR. Leaving the editor WITHOUT writing gives
#   "Nothing was written, so nothing was added." and keeps no draft.
#   After writing, skit goes straight to the review stage for the new draft.
#   Esc from review returns to the source stage, but the draft list is not refreshed there,
#   so no draft is listed yet. Leaving Add and entering it again lists it.
#   Ctrl+D arms the delete for whichever draft has keyboard focus. Tab is enough to give it
#   focus; the row does not have to be activated first. Ctrl+D typed while a text field has
#   focus still deletes a character instead, so the focus must be on the draft row.

Output "/out/walkthrough-drafts.mp4"
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

Type "a"
Wait+Screen /Ctrl.O/
Sleep 2s

# Ctrl+N writes a draft and opens it in vim.
Ctrl+N
Wait+Screen /skit-new-/
Sleep 3s
Screenshot "/out/d01-draft-in-editor.png"

# Write something, then save and quit. Quitting without writing keeps no draft.
Escape
Sleep 1s
Type "Go"
Sleep 500ms
Type "print('draft')"
Escape
Sleep 500ms
Type ":wq"
Enter

# Writing the draft routes straight to the review stage for it.
Wait+Screen /skit-new-/
Sleep 3s
Screenshot "/out/d02-draft-review.png"

# Leave Add, then enter it again so the source stage lists the kept draft.
Escape
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s
Type "a"
Wait+Screen /skit-new-/
Sleep 2s
Screenshot "/out/d03-source-lists-draft.png"

# Tab to the draft row. Focus is all the delete needs, so there is no Enter here.
# The footer on this frame advertises [Ctrl+D] Delete draft…
Tab 4
Sleep 3s
Screenshot "/out/d04-draft-focused.png"

# The PNG this beat writes can show a neighbouring beat instead of this one, and the frame
# before it can show this one early. That is an artifact of the VHS CAPTURE STACK, not of
# skit and not of this tape. Proof: in the same recording the emulator text grid holds the
# right screen at each beat, and the mp4 shows the correct screen second by second, while
# the rasterised PNG lands on the wrong side of the keystroke. A person at a real terminal
# sees the confirmation. Do not add keystrokes to "fix" it.
# No Wait can rescue the frame either: the draft name is already on screen in the Source
# path field, so any pattern naming the draft matches the neighbouring frame too. Sleep,
# and report.
Ctrl+D
Sleep 4s
Screenshot "/out/d05-draft-delete-confirm.png"

# Keep the draft. Esc cancels.
Escape
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s
Type "q"
Sleep 2s
