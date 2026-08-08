# Rust and Ratatui cutover

Status: **in review**. Version 0.5.0 is the first Rust-only release. The Python implementation and
Python development toolchain are removed. PyPI still distributes `skit-cli` as a Maturin binary
wheel so existing `uv tool` installation commands continue to work.

Do not read this document as a record of a completed release. It states the architecture, the
compatibility boundary, and the gates a release must pass. The pull request that proposes a release
carries the gate evidence.

## Architecture

```text
skit-domain          pure values and invariants
      ↑
skit-application     use cases, ports, errors, stable frontend data
      ↑              ↑
skit-store       skit-ui (pure reducer and serializable view model)
skit-runtime         ↑
      ↑          skit-tui (Ratatui adapter)
      └──────────────↑
                  skit-cli (composition root)

future: skit-tauri → skit-application + skit-ui
```

`skit-application` has no frontend, parser, TOML, or filesystem dependency. `skit-ui` has no
terminal dependency. It serializes actions, effects, screens, forms, and library state. Ratatui is
one adapter. A future Tauri crate can be a peer adapter and can use the same application ports.

## Compatibility boundary

The cutover reads the version 0.4 data, state, and configuration layout in place. It does not run a
bulk migration and does not rewrite data during startup.

- `SKIT_DATA_DIR/scripts/<slug>/meta.toml` remains authoritative.
- `registry.toml` remains a rebuildable projection. Stale rows fall back to metadata and repair in
  one nonblocking batch.
- Missing entry IDs remain valid legacy data. The first mutation stamps an ID under a lock.
- Unknown entry kinds and unknown TOML fields remain intact.
- Stored script bytes, line endings, and Unix permissions remain intact.
- Presets, last values, secret scrubbing, and state files keep the version 0.4 schema.
- Mutations use atomic replacement, identity checks, and source-hash compare-and-swap.
- One corrupt entry produces one diagnostic. It cannot hide other entries.
- A launched child keeps its exit code. Pre-launch errors keep codes 2, 125, 126, 127, and 130.

Compatibility tests use version 0.4 fixtures and run read, run, mutation, state, configuration, and
Agent Skill paths against them.

## Behavior changes from version 0.4

The product capabilities remain available, but some presentation behavior is different:

- The TUI now uses Ratatui and Crossterm. It does not use Textual or CSS.
- Library search uses deterministic case-insensitive substring matching. It does not use fuzzy
  ranking.
- Forms use one consistent text editor. Boolean, choice, numeric, and path fields show their type
  and validate at submission. The first Rust release does not show separate checkbox, choice-list,
  or file-browser widgets.
- The add screen is one direct form. Source analysis still runs, and the settings screen can manage
  detected fields after the add. The old multi-step review choreography is removed.
- Responsive layout is computed by Ratatui constraints. Narrow terminals stack the library panes.
- TUI actions keep keyboard and mouse paths. Footer chips and form rows are click targets.
- Human output remains localized. Machine JSON remains an English contract.
- A typed error carries a stable message template and its values. skit translates the template and
  then inserts the values. Version 0.4 translated rendered text by substring replacement, which
  could rewrite a user value or part of an English word.

These changes do not alter stored data. A user can install version 0.5 over version 0.4 and use the
same library without an export or import.

## Localized messages

`skit-i18n` owns one static catalog and two presentation types:

- `Message` holds a stable English template, its ordered values, and optional nested messages.
  `Message::localize` translates the template with an exact catalog lookup, then inserts the values
  unchanged. A value is user data, so it never reaches the translator.
- `Localize` is the trait each user-visible error implements. The `message` method matches on every
  variant, so a new variant needs a new template, and the compiler enforces that.

`skit-i18n/tests/catalog.rs` walks each crate's `src` tree, collects every `Message::new` template
outside `#[cfg(test)]` modules, and fails when the catalog has no complete row for one of them. Each
crate also has a `localization` test that builds every error variant and checks that the English
text matches the `thiserror` display, that each locale fills every hole, and that each value stays
byte-identical.

Clap composes its own usage report from skit-authored text. `skit-cli` translates the command tree
with exact lookups before parsing, so a token such as `--help` never changes. Only the framework
headings still use `skit_i18n::render`, which replaces a catalog row that is marked `composable`
and only at word boundaries.

## TDD and verification

Each port started with a failing contract test. Tests cover ordinary paths and important refusal,
corruption, rollback, concurrency, terminal, and process boundaries. A release must pass every gate
below, and the pull request must report the result of each one:

- Rustfmt and Clippy with warnings as errors;
- Rustdoc with warnings as errors;
- all workspace tests on Linux, macOS, and Windows;
- complete executable-source line coverage after LCOV records are merged;
- cargo-mutants with zero survivors;
- complete English, Simplified Chinese, and Traditional Chinese catalogs;
- cargo-deny, cargo-audit, and zizmor;
- Maturin wheel, source archive, and `uv tool install` smoke tests;
- release and macro benchmark budgets.

LLVM can assign counters to a closing brace, a blank line, a function signature, or the first line
of a covered multiline call. The coverage gate ignores only these structural mappings. It rejects
all uncovered executable source lines. Mutation testing independently checks behavior assertions.
