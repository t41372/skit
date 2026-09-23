# Terminal palette design

Status: owner-approved design, 2026-09-23. Phases 0 and 1 are done; see "Phases".

## Decision

The TUI gets two themes:

- `terminal` is the new default. It uses the terminal's own default foreground, default
  background, and ANSI palette. Style attributes (bold, dim, reverse) carry every state. A hue is
  only decoration.
- `skit` is the version 0.4 look: a fixed btop-style palette with a terracotta accent. It stays
  available as an opt-in theme.

`NO_COLOR` turns either theme into its monochrome variant. The owner made the two theme decisions
on 2026-09-23.

## Why

The current palette has these defects. They are not a matter of taste.

1. Body text, titles, and input values use `Color::White`, which is SGR 97 (bright white). Against
   the background, bright white measures 1.0:1 to 1.3:1 on every light profile in the table below.
   Version 0.4 is split here (`v0.4.0`):
   - Body text used `foreground="ansi_default"` (`src/skit/theme.py`). Bright white there is a
     porting defect.
   - Table headers (`theme.py:118`) and the library panel title (`src/skit/tui.py:276`) used
     `ansi_bright_white`. Bright white there is version 0.4 behavior.
2. Hints, suggestion tails, and the run form scrollbar use `Color::DarkGray`, which is SGR 90
   (bright black). It measures 1.0:1 on Solarized Dark (bright black is the background) and 2.4:1
   to 2.9:1 on Ghostty and Terminal.app dark. Version 0.4 used `[dim]` for secondary text
   (`tui.py:538-552`) and `#4A413C` for scrollbars (`theme.py:100`). Both differ from the port.
3. Ratatui and crossterm do not downgrade `Color::Rgb`. They always send `38;2;r;g;b`. Terminal.app
   supports 24-bit color only from macOS 26. Version 0.4 downgraded through Rich 15.0.0, so this is
   a version 0.4 parity gap.
4. `NO_COLOR` erases every state. Measured on 2026-09-23 with a real PTY (`expect`, 100x30,
   `TERM=xterm-256color`):
   - Without `NO_COLOR`, the selected library row is sent as
     `ESC[1m ESC[38;2;238;238;238;48;2;90;45;30m hello`.
   - With `NO_COLOR=1`, the same row is sent as `ESC[1m ESC[;m hello`.
   - crossterm 0.29 writes an empty color command when `NO_COLOR` is set. `ESC[;m` is SGR 0,
     which also resets bold. Ratatui writes modifiers before colors, so every modifier on a cell
     whose color changed is lost. Selection and focus use only color, so the selected row looks
     like every other row.
   - Version 0.4 (Textual 8.2.8) treats a present `NO_COLOR` as set, even when it is empty, and
     applies the `NoColor` filter to its ANSI theme. The filter sets every color to the default and
     keeps bold, reverse, and dim.
   - Every existing TUI PTY test sets `NO_COLOR=1` (`crates/skit-cli/tests/terminal_pty.rs:499`),
     so no end-to-end test has seen a color.

## Inventory

The production code in `skit-tui` has about 200 color and style sites. About 100 test assertions
compare concrete colors.

| Source | Sites |
| --- | --- |
| `theme.rs` constants | 7 truecolor values |
| `footer.rs:35-36` | 2 truecolor pill values |
| `run_modal.rs:964,992,995` | accent hex written inline, not from `theme.rs` |
| `Color::White` / `Color::DarkGray` | body text, titles, hints, scrollbars |
| `Modifier::DIM` | secondary text in add, library, picker |
| `session.rs:5554` `..SelectStyle::default()` | leaks ratatui-interact White, DarkGray |
| `ButtonStyle` defaults | pressed Black on White, disabled DarkGray |
| ratatui-interact `Toast` | hardcoded Black background, White text, Cyan border; no override |

## Measured contrast

WCAG contrast of each color against the default background. Values below 3.0 fail.

| Profile | fg | 6 cyan | 5 magenta | 2 green | 3 yellow | 8 | 15 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Ghostty | 14.0 | 6.8 | 5.2 | 7.0 | 8.7 | 2.4 | 11.6 |
| iTerm2, dark | 12.9 | 8.2 | 4.0 | 7.3 | 9.5 | 3.2 | 17.6 |
| iTerm2, light | 18.2 | 2.1 | 4.3 | 2.3 | 1.8 | 5.3 | 1.0 |
| Windows Terminal Campbell | 12.2 | 6.1 | 2.4 | 5.7 | 7.5 | 4.3 | 17.5 |
| Terminal.app Basic, light (*) | 21.0 | 2.96 | 5.9 | 3.3 | 3.0 | 5.7 | 1.2 |
| Terminal.app Basic, dark (*) | 16.7 | 5.6 | 2.8 | 5.1 | 5.5 | 2.9 | 13.4 |
| Solarized Dark | 4.7 | 4.8 | 3.3 | 4.7 | 4.7 | 1.0 | 13.9 |
| Solarized Light | 4.1 | 2.9 | 4.2 | 3.0 | 2.98 | 13.9 | 1.0 |

(*) The Terminal.app ANSI values come from an uncited Wikipedia column. Terminal.app can also adjust
colors when it draws them. Do not let these rows decide a choice.

Sources: Ghostty `v1.3.1` `src/terminal/color.zig:297-313` and `src/config/Config.zig:590-598`;
iTerm2 `plists/DefaultBookmark.plist`; Windows Terminal `defaults.json:129-150` (Campbell is the
default scheme for both light and dark); Solarized `README.md` 16-color mapping.

No hue reaches 3.0 on every profile. Cyan fails only on light backgrounds. Magenta fails on
Campbell, which every Windows Terminal user gets by default. Green and yellow fail on light
backgrounds too.

## Rules for the `terminal` theme

1. A hue never colors text that a person must read. The accent colors only borders, glyphs, and
   markers. Status colors (green, yellow, red) color only the ✓ ⚠ ✗ glyph. The text next to it
   uses the default foreground.
2. Never use ANSI black or white as a foreground. Use the default foreground.
3. Never set a background except through `REVERSED`. The terminal then guarantees the contrast.
4. A hue never carries a state alone. Every state must survive the monochrome variant.

## Roles

Screens stop naming colors. Each draws with a role. A role has two parts, because many
ratatui-interact widgets accept only a `Color` pair and add only `BOLD` on their own:

- `colors`: the foreground and background that a widget constructor accepts.
- `patch`: a `Style` that the screen applies to the area the widget returns, with
  `Buffer::set_style`. The patch adds `REVERSED`, `BOLD`, or `DIM` and can force colors back to
  `Reset`.

| Role | `terminal` | monochrome | `skit` |
| --- | --- | --- | --- |
| text | default fg | default fg | default fg |
| title, table header | default fg, bold | bold | ANSI bright white, bold (version 0.4) |
| scrollbar | dim | dim | `#4A413C` (version 0.4) |
| muted | dim | dim | dim |
| emphasis (entry name, section heading) | default fg, bold | bold | `#D97757`, bold |
| border, idle | dim | dim | `#3A3A3A` |
| border, focused | accent, not dim | not dim | `#D97757` |
| selection | reverse, bold | reverse, bold | `#EEEEEE` on `#5A2D1E`, bold |
| selection, unfocused list | bold, marker glyph | bold, marker glyph | as today |
| caret in text area | reverse | reverse | black on `#D97757` |
| footer keycap | accent fg + reverse, bold | reverse, bold | see "Footer chips" |
| status glyph | ANSI green / yellow / red | glyph only | ANSI green / yellow / red |
| status text | default fg | default fg | as today |
| panel tint | none; titles name the panel | none | per-panel truecolor as today |

## Accent hue

Decision (delegated by the owner, 2026-09-23): pick the hue from the background. The accent is
decoration only, so a weak hue costs less, but no single hue works on every default profile:

- Dark background: ANSI 6 (cyan). Lowest measured value 4.8.
- Light background: ANSI 5 (magenta). Lowest value in the table 4.2 (Solarized Light). Terminal.app's
  Clear Light profile, which is not in the table, measures 3.9.
- Unknown background: no hue. The accent role falls back to "not dim".

skit learns the background with an OSC 11 query at startup, the same method as bat, yazi, Neovim,
Codex, and Claude Code. The costs:

- A new dependency (for example `terminal-colorsaurus`) must pass `cargo deny` and `cargo audit`,
  or skit writes the query itself.
- The query must finish before crossterm's event reader starts, or the reply arrives as input.
- The query has a timeout. `terminal-colorsaurus` uses 1 second by default and sends DA1 so that a
  terminal without OSC 11 answers early.
- The PTY harness must answer OSC 10 and OSC 11 the way it answers `ESC[6n` today. Otherwise every
  PTY test waits for the timeout.
- GNU Screen, the Linux console, and mosh do not answer. They get the unknown-background fallback.

## Footer chips

Today each chip has one style: the key and the label are both terracotta over a faint dark-brown
box. That box is the only sign that the chip is a button.

| Candidate | Key | Edge under `NO_COLOR` | Key contrast |
| --- | --- | --- | --- |
| A | accent, bold text | lost | accent vs background; fails rule 1 |
| B | accent reversed keycap | kept | accent vs background; fails rule 1 |
| B′ | default reversed keycap | kept | fg vs background: 4.1 or more everywhere |

Decision (owner, 2026-09-23): B. It is the function-bar pattern of htop and Midnight Commander.
no-color.org allows reverse for a status bar. The mockup artifact shows all candidates on every
profile.

B is the one exception to rule 1, and the background-based accent makes it safe:

- Dark background: the cyan keycap measures 4.8 or more.
- Light background: the magenta keycap measures 4.2 or more in the table (3.9 on Clear Light).
- Unknown background or monochrome: the keycap reverses the default foreground, which is B′
  (4.1 or more everywhere).

ratatui-interact `Button` renders `" {icon} {label} "` with one `Style`, so a split key and label
style cannot go through `Button`. skit draws its own two-span chip and registers the same click
region.

Decision: pending owner review of the mockup.

## Widgets that need a post-pass

`REVERSED` cannot reach these widgets through their style APIs. The `NO_COLOR` filter removes
their colors, but it cannot add an attribute:

- radio buttons (`ButtonStyle` toggle colors)
- modal buttons and the run chip (`ButtonStyle::focused(fg, bg)`)
- `CheckBoxStyle` and `SelectStyle` borders
- `Toast`, which has no override at all

These widgets render with `Reset` colors. Then the screen applies the role's `patch` to the area
that the widget returns. `Toast` gets the same post-pass, or skit draws its own toast.

## Theme selection

skit-cli selects the palette in the composition root. skit-tui never reads the environment.

1. `NO_COLOR` wins over the theme. The TUI treats a present variable as set, even when it is empty
   (Textual 8.2.8 `app.py:614-616`). The CLI keeps its non-empty check (Rich 15.0.0). Both are
   version 0.4 behavior; the difference is deliberate.
2. Otherwise the `theme` config key selects `terminal` (default) or `skit`.
3. The `skit` theme picks its color depth with Rich 15.0.0 `_detect_color_system`
   (`console.py:789-811`), ported exactly:
   - Unix: `COLORTERM` in `truecolor`, `24bit` gives 24-bit. Else the `TERM` suffix after the last
     hyphen: `kitty` and `256color` give 256 colors, `16color` and every other suffix give 16.
   - Windows: Textual passes `legacy_windows=False`, so VT processing on Windows 10 build 15063 or
     later gives 24-bit. Rich reads no `WT_SESSION` variable.
   - The 256-color and 16-color forms of the fixed colors are precomputed with Rich's
     `Color.downgrade` (`color.py:512-568`) and stored as constants. Example: `#D97757` becomes
     index 173 and bright red.
4. `NO_COLOR` is an output filter, as in version 0.4 (Textual applies `NoColor` to its output).
   A backend wrapper (`AppearanceBackend` in `crates/skit-tui/src/appearance.rs`) sets the
   foreground, background, and underline color of every cell it writes to the default and keeps
   every attribute. Ratatui compares frames before the wrapper, with their colors, so the terminal
   receives the same cell writes as in a colored session.
   - A filter on the frame buffer was tried first and refused: it made Ratatui skip cells whose
     only change was a color. An accepted path suggestion then never redrew, and four existing PTY
     tests in `crates/skit-cli/tests/terminal_pty.rs` failed. The monochrome
   variant of a theme is that theme plus the filter. Third-party widget colors (`Toast`,
   `SelectStyle`) are removed the same way. The filter runs only when `NO_COLOR` is set.
   - skit also calls `crossterm::style::force_color_output(true)` for the session, so crossterm no
     longer writes `ESC[;m`, which erased the attributes. The session restores the previous value
     on exit.
   - On 2026-09-23 the only production draws were `terminal.rs:369` and `terminal.rs:649`, both
     in skit-tui, and no crate outside Ratatui sent crossterm style commands. The CLI's line output
     writes its own SGR strings (`paint_for_output` in `crates/skit-cli/src/cli.rs`), so neither
     change reaches it.
   - skit-cli passes the decision to skit-tui as an `Appearance` value. Phase 5 adds the theme,
     the color depth, and the background to the same value.
5. Every entry point that builds a terminal goes through the same selection. A Preferences save
   that changes the theme repaints the running session.

## Configuration and surfaces

- `CONFIG_KEYS` gets `theme` with the values `terminal` and `skit`. Version 0.4 ignores an unknown
  top-level key and keeps it on save (`src/skit/config.py:91-103` at `v0.4.0`), so a downgrade is
  safe.
- `skit config theme [VALUE]` with `--json`, deterministic exit codes, and dynamic completion.
- A Preferences control, with English, Simplified Chinese, and Traditional Chinese copy.
- `skills/skit/SKILL.md` lists the new key.

## Scope

The CLI's line output keeps version 0.4's Rich colors (`[green]`, `[yellow]`, `[red]`, `[dim]`)
for parity. Yellow text fails on light backgrounds, but this design leaves it alone on purpose.

## Records

A restoration and an addition are different things in this repository. Keep them apart.

- `docs/parity-backlog.md` gets only real version 0.4 gaps, each with its oracle line and an
  end-to-end test that pins the version 0.4 result:
  - body text and input values in the default foreground (`theme.py` `foreground="ansi_default"`)
  - secondary text in dim (`tui.py:538-552`)
  - scrollbars in `#4A413C` (`theme.py:100`)
  - `NO_COLOR`: colors to default, bold, reverse, and dim kept (Textual 8.2.8 `NoColor`)
  - color depth (Rich 15.0.0)
  Before phase 2, map each `Color::White` and `Color::DarkGray` site to its version 0.4
  counterpart. Only a true match is a restoration.
- `docs/behavior-changes.md` gets every addition:
  - the `terminal` theme and the default switch (owner decision)
  - in monochrome: reverse selection and dim idle borders (version 0.4 left the selected row bold
    only and every border in the default color)
  - titles and table headers in the default foreground in the `terminal` theme
  - the `skit` theme stays, so nothing is removed.

## Guardrails

- `clippy.toml` `disallowed-methods`: `Color::Rgb`, `Color::Indexed`, and `Style::fg`/`Style::bg`
  outside `theme.rs`, which allows them locally. Codex CLI uses the same lint for the same reason.
- Every palette role must keep a visible state in the monochrome variant.

## Tests

Owner ruling, 2026-09-23:

- End-to-end tests are the main mechanism. Each one ends with an artifact that a rerun reproduces
  byte for byte.
- An isolated test is added only where a gate requires it, and only after a written list of the
  ways the unit can fail.
- The mutation gate is deferred. Write as many tests as possible now; add tests later where a gate
  falls short.
- The old concrete-color unit assertions stay while they pass. When a phase changes a color that
  one of them checks, ask the owner whether the census replaces it.

The artifact:

- A PTY run of the real binary, parsed with `vt100` 0.16.2 (`crates/skit-cli/tests/support/pty.rs`).
  `vt100::Cell` reports foreground, background, bold, dim, and inverse, so the census sees every
  attribute this design uses.
- For each screen, theme, and environment, the final frame as a sorted JSON census of cells: text,
  foreground, background, and attributes. The environments:
  - color: default, `NO_COLOR=1`, no `COLORTERM`
  - background: the harness answers OSC 11 dark, answers OSC 11 light, or answers only DA1 (the
    fast unknown-background path, which must not wait for the timeout)
- A rerun must produce the same bytes.
- The child keeps `LLVM_PROFILE_FILE` and `CARGO_LLVM_COV*`, and each session ends with two
  Ctrl+C presses. A killed child writes no coverage profile.
- Human review: the demo pipeline with one pinned dark VHS theme and one pinned light VHS theme.

## Phases

Each phase after the baseline changes the census in one known way. Version 0.4 parity comes
first, because the output filter does not depend on the role refactor.

0. Baseline (done 2026-09-23). The census harness, 24 committed census files (8 screens, 3
   environments), and two failing E2E tests that pin version 0.4: under `NO_COLOR=1` the selected
   row keeps bold and no cell has a color; under an empty `NO_COLOR` no cell has a color.
1. `NO_COLOR` parity (done 2026-09-23): the `Appearance` value, the output filter, and
   `force_color_output`. Only the 8 `no-color` census files changed, and a cell-by-cell check
   showed that only attributes changed (bold and dim returned; no text, foreground, or background
   moved). The phase 0 tests pass. Known coverage gap: three forwarding methods of
   `AppearanceBackend` (`append_lines`, `get_cursor_position`, `window_size`, 9 lines) are never
   called by a full-screen session, so no end-to-end test reaches them. By the owner's ruling, an
   isolated test waits until the coverage gate asks for it.
2. Role refactor. The census stays identical. Before it starts, the census must reach every
   production color site, or each unreached site is written down as unreachable. On 2026-09-23
   the 8-screen census reached 49 of 192 sites. Screens with an absolute temporary path stay out,
   because macOS and Linux temporary paths have different lengths. The `skit` roles resolve to
   today's constants (`ACCENT`, `SELECT_BG`, and the others), so the old assertions keep passing.
   The palette travels with `locale`, not through a global.
3. Parity restorations, after the site mapping: body text to the default foreground, secondary
   text to dim, scrollbars to `#4A413C`. The census changes only those cells.
4. `skit` theme color depth. Only the `no-colorterm` census changes, and no cell in it keeps a
   `#rrggbb` color. This phase follows the role refactor on purpose: each palette entry then holds
   its 24-bit, 256-color, and 16-color forms, so no color can reach the output without a mapping.
5. `terminal` theme with the OSC 11 accent, the footer chip, the config key, the Preferences
   control, the CLI, and i18n. Then the default switches. The reverse selection under `NO_COLOR`
   is tested here, as an addition. When `Appearance` gains a field, `with_no_color` must build
   `Self { no_color, ..self }`, or it drops the theme.
6. Documentation, records, and demo assets.

At the end of each wave, the Linux prompt reruns the census on Linux and diffs it against the
committed files, and runs the walker corpus.
