# walkthrough-modals.tape — the overlays walkthrough-shots.tape does not reach.
#
# Kept separate on purpose: walkthrough-shots.tape is the tape that gets recorded twice and
# compared, so it is kept short and each new state added there costs that comparison a
# re-record. This file adds the pickers, editors and guards, each of which needs several
# keystrokes of setup, and it is not part of the comparison.
#
# Token menu order, read off w06-token-menu.png, counting from the first row:
#   1 Directory at run time {cwd}   2 This directory, as a fixed path   3 Today's date
#   4 Current time                  5 Home directory                    6 Environment variable…
#   7 File or folder…
# So Down 5 reaches the environment picker and Down 6 reaches the file picker.

Output "/out/walkthrough-modals.mp4"
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
Up 4
Sleep 1s

# ---------------------------------------------------------------- add: the Kind stage

# names.txt has no shebang and an extension skit does not map to a runtime, so the add
# workflow cannot infer the kind and stops to ask. This stage has no key of its own: it is
# reached only by giving it a source it cannot classify.
Type "a"
Sleep 2s
Type "names.txt"
Sleep 1s
Enter
# The source inspection is asynchronous and can take more than three seconds, so wait for
# the kind list rather than sleeping. Language names are not translated, so /PowerShell/ is
# a locale-invariant anchor.
Wait+Screen /PowerShell/
Sleep 2s
Screenshot "/out/m01-add-kind-stage.png"
Escape
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s

# ---------------------------------------------------------------- run form pickers

Enter
Wait+Screen /World/
Sleep 2s

# Ctrl+T then the last row opens the filesystem picker.
Ctrl+T
Wait+Screen /cwd/
Sleep 2s
Down 6
Sleep 1s
Enter
Wait+Screen /greet.py/
Sleep 2s
Screenshot "/out/m02-file-picker.png"
Escape
Sleep 2s

# Ctrl+T then "Environment variable…" chains into the environment picker.
Ctrl+T
Wait+Screen /cwd/
Sleep 2s
Down 5
Sleep 1s
Enter
Wait+Screen /PATH/
Sleep 2s
Screenshot "/out/m03-environment-picker.png"
Escape
Sleep 2s

Escape
Wait+Screen /greet/
Sleep 2s

# ---------------------------------------------------------------- the unsaved-changes guard

# Entry settings opens with the Name field focused. Typing makes the screen dirty, so Esc
# raises the guard instead of leaving. The guard takes y only; Enter is deliberately unbound
# so a reflex cannot throw the edit away.
Type "p"
Wait+Screen /greet.py/
Sleep 2s
Type "X"
Sleep 2s
Escape
Sleep 3s
Screenshot "/out/m05-confirm-discard.png"
# n keeps editing.
Type "n"
Sleep 2s
Escape
Sleep 2s
# y discards and leaves.
Type "y"
Wait+Screen /greet/
Sleep 2s

# ---------------------------------------------------------------- preferences: agent editor and skill picker

Type ","
Wait+Screen /VISUAL/
Sleep 2s

# On Linux, five Tabs move from Language to the agent list.
Tab 5
Sleep 2s
Enter
# The command label contains aider in every locale. The agent list does not.
Wait+Screen /aider/
Sleep 2s
Screenshot "/out/m04-runner-editor.png"
Sleep 1s
Escape
Sleep 2s

# Move past New agent to the Agent Skill door. The demo has no agent directories.
Tab 2
Enter
# This command fragment stays on one line in every locale and appears only in the picker.
Wait+Screen /agent install/
Sleep 2s
Screenshot "/out/m08-agent-skill-picker.png"
Sleep 1s
Escape
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s

# ---------------------------------------------------------------- the quit guard

# The first Ctrl+C arms a two-second toast. It is the only quit path that asks twice.
Ctrl+C
Sleep 1s
Screenshot "/out/m09-ctrl-c-toast.png"
Sleep 3s
Type "q"
Sleep 2s
