# Prompt entry design

This document describes the Rust 0.5 prompt implementation. Version 0.4 Python and Textual design
records remain available in Git history.

## Data and rendering

A prompt entry stores text as a copied or referenced payload. Managed `{{placeholders}}` use the
shared parameter model and frontend-neutral form plan. The renderer performs raw substitution. It
does not invoke a shell and does not add shell quoting or prompt escape syntax. Unmanaged text and
unmanaged `{{holes}}` pass through unchanged.

skit manages all detected placeholders when there are 30 or fewer. When it detects more than 30,
it manages none by default. The user can change the managed set in Entry settings or with
`skit params`.

## Runners

Prompt runners are argv arrays in `config.toml`. Each valid runner has a name and contains
`{{prompt}}` exactly once after the program. Seed rows cover claude, codex, opencode, amp,
antigravity, copilot, cursor, and pi. User rows remain authoritative and unknown fields survive
configuration writes.

Interactive forms make configured runners discoverable. Non-interactive selection is
`--runner`, then the entry pin, then exit 126. skit does not rank or guess a runner. Pi receives
one leading newline only when its opening text could be parsed as one of Pi's command forms; skit
reports that compatibility change.

## Delivery and limits

The rendered prompt is one process argument at the `{{prompt}}` position. Other runner arguments
stay separate. POSIX uses the platform argv interface. Windows uses the documented command-line
encoding rules. A typed error reports a rendered argv that exceeds the platform limit.

Prompt fields marked secret are never saved in last-used values, presets, or run history. The
rendered text is not a secret channel because the receiving agent can record it in session logs.

## Frontends and tests

`skit-ui` owns prompt form state and actions. `skit-tui` maps keyboard and mouse input to those
actions. The CLI exposes prompt add, inspection, parameter management, runner management, dry-run,
and launch behavior with stable JSON and exit codes.

Tests cover raw substitution, placeholder drift, runner validation, malformed configuration rows,
non-interactive resolution, Pi compatibility, Windows argument encoding, secret-state scrubbing,
and copied and referenced payload damage.
