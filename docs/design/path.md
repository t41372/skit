# Path value design

This document describes the Rust 0.5 path contract. Version 0.4 Textual design records remain
available in Git history.

## Model

`path` is one of the six parameter types. It is a path-shaped string, not a filesystem capability
or a security boundary. A declaration can select it with
`skit params NAME --type FIELD=path`. Static readers can also report it when the source parser has
enough information.

Path fields use the same frontend-neutral form model as other values. Ratatui edits their text,
and the CLI accepts them through `--set`. The domain does not require the path to exist. The child
program keeps its operating-system and language-runtime semantics.

## Expansion

Value-token expansion happens in the application pipeline. A leading `~`, `{cwd}`, and
`{env:NAME}` can produce path text. Glob expansion applies only to values whose declaration allows
multiple items and to extra argument tails. A single-value path stays one value.

The dry-run path uses the same prepared values and process plan as a real launch. It prints the
result without starting the child or writing run state.

## Compatibility

Stored `type = "path"` values remain in metadata and JSON output. Older or unknown parameter shapes
degrade to readable text where the version 0.4 contract requires that behavior. Source files in
`tests/corpus/` remain byte-exact.
