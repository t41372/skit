# Multi-language architecture

This document describes the Rust 0.5 implementation. Version 0.4 design records remain available
in Git history.

## Boundaries

- `skit-domain` owns open entry kinds and the closed parameter value model.
- `skit-language` owns static analysis, managed source blocks, injection, normalization, and
  prompt rendering. One parser layer serves analysis and source edits for each parsed language.
- `skit-runtime` builds process plans and starts programs. Launch paths do not depend on parser
  crates.
- `skit-form` converts parameter declarations into frontend-neutral form plans.
- `skit-ui` owns serializable state and reducer actions. Ratatui is an adapter in `skit-tui`.
- `skit-cli` is the composition root. It exposes every TUI capability through deterministic
  commands.

Dependency direction points toward domain and application. Frontends do not own product rules.

## Entry kinds

Entry kinds are open strings so a newer writer does not make older data unreadable. Known kinds
include Python, shell, JavaScript, TypeScript, fish, PowerShell, Ruby, Perl, Lua, R, executables,
command templates, and prompts. Unknown kinds stay visible and fail with a typed launch error.

Each kind has explicit analysis and runtime behavior. Runtime selection does not rank or guess
when the contract requires a pin. A non-interactive prompt run resolves `--runner`, then the entry
pin, then exits with code 126.

## Parameters and source edits

`ParamDecl` is the shared model. Its defaults are strings, signed integers, finite floats, or
Booleans. Metadata can also describe choices, multiple values, repeated flags, environment
targets, Boolean actions, prompts, help text, and secret storage policy.

Analyzers read source without executing it. Managed source edits change only the supported comment
block. `--normalize` is the only opt-in semantic edit: it can convert one shell constant to the
environment-default form. A reference entry never changes the original file through a copy
operation.

## Compatibility and tests

Version 0.4 data is authoritative. Reads keep unknown TOML fields, source bytes, permissions,
presets, state, and open entry kinds. Writes use identity and source-hash checks. Store changes use
locks, staging, atomic replacement, and rollback tests.

The language corpus in `tests/corpus/` is byte-exact. The release gates include all-feature tests,
complete executable-source line coverage, Clippy and Rustdoc warnings as errors, zero surviving
mutants, complete localization, and documentation and packaging builds.
