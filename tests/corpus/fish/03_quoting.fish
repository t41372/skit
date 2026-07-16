#!/usr/bin/env fish
# Quoting edges: a semicolon inside quotes must not split; comments after code; escaped chars.

set -q GREETING; or set GREETING 'hello; world'   # the ; is inside quotes, not a separator
set -q PROMPT; or set PROMPT "enter value:"       # a double-quoted literal
set -q PATTERN; or set PATTERN \*.txt             # an escaped glob char stays literal

# A full-line comment, then a bare word with an inline # that is NOT a comment.
echo done#notacomment
