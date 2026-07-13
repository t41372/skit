#!/usr/bin/env fish
# A script whose CLI surface is fish's builtin argparse — read into flag-delivery fields.

argparse -n deploy 'h/help' 'c/city=' 'r/retries=?' 'f/file=+' 'dry-run' 'v/verbose' -- $argv
or return

if set -q _flag_help
    echo "usage: deploy --city NAME [--retries N] [--file F ...] [--dry-run]"
    return 0
end

echo "deploying to $_flag_city"
