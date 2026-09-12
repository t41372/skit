Output "/out/demo.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 60ms

Hide
Type "source ~/.bashrc" Enter
Type "clear" Enter
Show

Type "bat greet.py" Enter
Sleep 3s
Type "clear" Enter
Sleep 500ms

Type "skit add greet.py --no-input" Enter
Sleep 2s
Type "skit" Enter
Sleep 2s

Enter
Sleep 2s
Ctrl+S
Sleep 4s

Type "skit" Enter
Sleep 2s
Type "s"
Sleep 2s
Escape
Sleep 1s
Type "q"
Sleep 1s
