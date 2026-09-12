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

# Ctrl+N is inert here, and correctly so: greet is a script, so its form has no Runner
# field and NewRunner is not enabled. The runner editor is recorded from the Agents screen
# below, where Ctrl+N is always offered.

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

# ---------------------------------------------------------------- agents: actions and removal

Type "R"
Wait+Screen /codex/
Sleep 2s

# Ctrl+N opens the runner (agent) editor.
# The anchor is the example name in the Name placeholder, "Name, e.g. aider". Both Chinese
# catalogs keep "aider" untranslated, and no configured agent has that name, so it cannot
# match the list underneath. A pattern naming {{prompt}} would match that list at once, so
# the wait would return before the editor drew and prove nothing.
Ctrl+N
Wait+Screen /aider/
Sleep 2s
Screenshot "/out/m04-runner-editor.png"
Escape
Wait+Screen /codex/
Sleep 2s

Enter
Sleep 3s
Screenshot "/out/m06-agent-row-actions.png"
Type "d"
Sleep 3s
Screenshot "/out/m07-agent-removal-confirm.png"
# n keeps the agent.
Type "n"
Sleep 2s
Escape
Wait+Screen /greet/
Sleep 2s

# ---------------------------------------------------------------- preferences: Ctrl+K picker

# Ctrl+K offers to install the skit skill into an AI agent's directory. The recording
# container has no ~/.claude, ~/.codex or ./.agents, so the picker has nothing to list and
# shows its empty state instead: it names the directories it looked in and gives the command
# to install by hand. That sentence is translated, but the command inside it is not, so
# "agent install" anchors the beat in every locale and appears nowhere on Preferences.
Type ","
Wait+Screen /VISUAL/
Sleep 2s
Ctrl+K
Wait+Screen /agent install/
Sleep 2s
Screenshot "/out/m08-agent-skill-picker.png"
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
