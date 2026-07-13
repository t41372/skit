#!/usr/bin/env fish
# Block-depth tracking: idioms inside function/if/while/for/begin/switch are NOT top-level.

set -q TOP; or set TOP 1          # top level — detected

function _configure
    set -q INNER; or set INNER 2  # inside a function — not detected
end

for host in web1 web2
    set -q PER_HOST; or set PER_HOST 3   # inside a for — not detected
end

if test -n "$TOP"
    begin
        set -q DEEP; or set DEEP 4       # nested begin inside if — not detected
    end
end

switch $TOP
    case 1
        set -q CASED; or set CASED 5     # inside switch — not detected
end

set -q ALSO_TOP; or set ALSO_TOP 6   # back at top level — detected
