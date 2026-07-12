# Recording-shell niceties (appended to /root/.bashrc in the demo image).
# The tape sources this so the prompt + bat defaults are set before anything is shown.

# bat: no pager (it must not launch `less` inside the tape), line numbers + filename header,
# a soft dark theme that reads well on the recording's black background.
export BAT_PAGING=never
export BAT_STYLE=numbers,header
export BAT_THEME=OneHalfDark

# The editor skit's "edit script" (`e`) opens. vim is installed in the image and configured
# by /root/.vimrc (docs/assets/demo/demo.vimrc), so the edit scene shows a real setup.
export EDITOR=vim

# A single warm accent prompt (skit's #d97757 ≈ 256-color 173), bold. `»` is Latin-1, so it
# renders in JetBrains Mono — no glyph gamble, and no non-ASCII typed into the shell.
PS1='\[\e[1;38;5;173m\]» \[\e[0m\]'
