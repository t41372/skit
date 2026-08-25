Output "/out/shots.mp4"
Require skit

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 20
Set Width 1280
Set Height 780
Set Padding 20
Set TypingSpeed 40ms

Hide
Type "source ~/.bashrc" Enter
Type "skit add greet.py --no-input" Enter
Type "clear" Enter
Type "skit" Enter
Sleep 2s
Show

Screenshot "/out/shot-library.png"
Sleep 2s

Enter
Sleep 2s
Screenshot "/out/shot-form.png"
Sleep 2s
Escape
Sleep 1s

Ctrl+N
Sleep 2s
Screenshot "/out/shot-add.png"
Sleep 2s
Escape
Sleep 1s

Type "s"
Sleep 2s
Screenshot "/out/shot-settings.png"
Sleep 2s
Escape
Sleep 1s
Type "q"
