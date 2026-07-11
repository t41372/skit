# Recording-shell niceties (appended to /root/.bashrc in the demo image).
# The tape sources this so the prompt + bat defaults are set before anything is shown.

# bat: no pager (it must not launch `less` inside the tape), line numbers + filename header,
# a soft dark theme that reads well on the recording's black background.
export BAT_PAGING=never
export BAT_STYLE=numbers,header
export BAT_THEME=OneHalfDark

# A single warm accent prompt (skit's #d97757 ≈ 256-color 173), bold. `»` is Latin-1, so it
# renders in JetBrains Mono — no glyph gamble, and no non-ASCII typed into the shell.
PS1='\[\e[1;38;5;173m\]» \[\e[0m\]'
