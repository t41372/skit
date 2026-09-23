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

The source of truth is the latest Python version 0.4 development revision on `main`, not the
version 0.4.0 release tag. This review is pinned to
`origin/main@206f9ef946fc45835cb2479593794431f2620c32`. The cutover reads that revision's data,
state, and configuration layout in place. It does not run a bulk migration and does not rewrite
data during startup.

- `SKIT_DATA_DIR/scripts/<slug>/meta.toml` remains authoritative.
- `registry.toml` remains a rebuildable projection. Stale rows fall back to metadata. Reads never
  repair it. Mutations maintain it, and `doctor --rebuild` repairs it explicitly.
- Missing entry IDs remain valid legacy data. The first mutation stamps an ID under a lock.
- Unknown entry kinds and unknown TOML fields remain intact.
- Stored script bytes, line endings, and Unix permissions remain intact.
- Presets, last values, secret scrubbing, and state files keep the version 0.4 schema.
- Mutations use atomic replacement, identity checks, and source-hash compare-and-swap.
- One corrupt entry produces one diagnostic. It cannot hide other entries.
- A launched child keeps its exit code. Management failures keep 1/2. Run preflight keeps 2, 125,
  126, and 127. An interactive cancellation keeps the established command-specific status.

Compatibility tests use fixtures and executable behavior from the pinned Python revision. They run
read, run, mutation, state, configuration, and Agent Skill paths against them.

## Superset contract

Version 0.5 must be a strict product superset of the pinned latest Python revision. A presentation
rewrite cannot remove a control, workflow, shortcut, discovery path, parser result, launch
behavior, machine record, or data-recovery path. Additive keys and capabilities are permitted only
when all existing paths continue to work.

The permitted presentation changes are narrow:

- Ratatui and Crossterm replace Textual and CSS.
- Ratatui constraints replace CSS layout rules while preserving responsive reachability.
- Visual styling can change without changing available information or actions.
- Typed localization inserts user values after translating the exact message template.

The following items were previously described here as presentation changes. That classification
was incorrect. They are required product capabilities and are release blockers until restored:

- deterministic case-insensitive subsequence search, activity sorting, rerun, help, and complete
  entry details;
- checkbox, choice, numeric, secret, path, argument-list, and read-only form semantics;
- file, environment, token, candidate, preset, and runner pickers;
- the staged add/review/draft, settings, preferences, health, runner, and preset workflows;
- parser-backed source analysis, CLI reflection, reconciliation, precise injection, and
  normalization;
- all stable JSON shapes, exit statuses, configuration behavior, and state semantics.

The executable status and evidence for these areas is in `rust-contract-matrix.md`. This document
does not claim that the cutover is complete while any matrix row is incomplete.

## Ratatui layout and input contracts

The Ratatui adapter classifies each frame once. A width below 80 columns is narrow. Heights 0-9
are tiny, 10-15 are short, 16-27 are normal, and 28 or more are tall. The root planner gives the
primary body its minimum rows before it assigns header and footer rows. Short and tiny layouts use
a flat Library search row and an undecorated footer. Self-titled screens and modals own their title;
they do not also render a global title. Boundary tests cover zero-size viewports, each adjacent tier,
all three locales, hit containment, and body growth that never shrinks the primary viewport.

Mouse actions use one semantic press-and-release contract. Primary Down arms the target. A primary
Up activates it only when the semantic target still matches. A drag, a nonprimary button, a resize,
or a different owner cancels the press. An editable field can place its caret on primary Down, but
that placement does not activate an action. Shared geometry maps terminal cells to complete ASCII,
wide, combining, emoji-ZWJ, secret, and horizontally scrolled graphemes. Wheel input belongs to the
topmost visible list, dropdown, modal, body, or footer under the pointer. It cancels an armed click
and clamps the owner's scroll offset to its final visual rows.

The live Unix PTY test sends real SGR mouse Down and Up bytes at 46x12. It verifies the 1003 and 1006
mode transitions and their shutdown restoration. The test replays all output through `vt100` and checks
the final terminal grid, so erased history text cannot pass as visible text. Ignored pointer motion
emits no frame. Consumed input, reducer actions, resize events, and completed background work request
a redraw. The dirty flag uses a monotonic OR until the loop draws the next frame.

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
