# Terminal palette design

Status: owner-approved design, 2026-09-23. Phases 0 to 5 are done, and phase 6 is in progress; see "Phases".

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


## Role counterparts

The `skit` column is the value today. Phase 3 changes a role only where version 0.4 differs.

| Role | `skit` today | Version 0.4 | Phase 3 |
| --- | --- | --- | --- |
| `text` | `White` (SGR 97) | `foreground="ansi_default"` (`theme.py`) | default foreground |
| `title` | `White`, bold | border title `ansi_bright_white` (`tui.py:276`) | keep |
| `table_header` | `White`, bold | `ansi_bright_white`, bold (`theme.py:118`) | keep |
| `heading` | accent, bold | `.section { color: $accent }` (`tui_prefs.py:139`) | keep |
| `required` | accent, bold | `[$accent]required` (`tui_form.py:195`) | check bold |
| `emphasis` | accent, bold | `[bold $accent]` entry name (`tui.py:534`) | keep |
| `hint` | `DarkGray` (SGR 90) | `[dim]` (`tui.py:538-552`) | dim |
| `muted` | dim | `[dim]` | keep |
| `suggestion` | `DarkGray` | not checked | check |
| `scrollbar` | run form `DarkGray`, settings indigo | `scrollbar: #4A413C` (`theme.py:100`) | `#4A413C` |
| `key_hint`, `marker`, `notice` | accent | `$accent` | keep |
| `status` | ANSI green, yellow, red | `success`, `warning`, `error` ANSI (`theme.py`) | keep |
| `border` | accent or `#3A3A3A` | `$accent` or `border-blurred #3A3A3A` | keep |
| `panel_color` | per-panel tints | `BOX_*` tints (`theme.py`) | keep |
| `selection` | `#EEEEEE` on `#5A2D1E` | `block-cursor` colors (`theme.py`) | keep |
| `caret` | black on accent | `input-cursor` bright white on black (`theme.py`, one-line inputs) | keep: the Rust text area draws its own cursor cell, and one-line inputs use the terminal cursor |
| picker, select, checkbox, radio, button, chip styles | accent and selection colors | Textual widgets with the theme above | keep |

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

## Widgets that need a post-pass

`REVERSED` cannot reach these widgets through their style APIs. The `NO_COLOR` filter removes
their colors, but it cannot add an attribute:

- radio buttons (`ButtonStyle` toggle colors)
- modal buttons and the run chip (`ButtonStyle::focused(fg, bg)`)
- `CheckBoxStyle` and `SelectStyle` borders
- `Toast`, which has no override at all

These widgets render with `Reset` colors. Then the screen applies the role's `patch` to the area
that the widget returns. `Toast` gets the same post-pass, or skit draws its own toast.

Buttons (added 2026-09-23, after the branch review). In the `skit` theme a pill background is the
only sign that a word is a button. Without it, an idle `▾ insert` chip, a dialog button, or a
Preferences action reads as plain text, which is the "reachable is not discoverable" failure of the
first path picker. `patch_button` therefore marks both states:

- A focused button is reversed.
- An idle button shows `[` and `]` in its two padding cells: `[Discard]`, `[▾ insert]`. This is the
  button mark of `dialog`, `whiptail`, and Midnight Commander. It needs no color, it survives
  `NO_COLOR`, and it does not change a width.
- A button with a key, such as an agent row's `Enter Edit`, is a keycap, as a footer chip is.

`patch_button` takes the painted cells, not the click region. ratatui-interact sizes a `Button`
region by characters and a `PopupDialog` region by bytes plus four, so both regions miss the
painted cells of a wide or non-ASCII label.

Other roles that the review added:

- `bold_text` removes `DIM`. A title sits on a dim panel border, and a cell with both is faint on
  most terminals.
- `Panel::Overlay` is a picker drawn over another screen. It holds the focus, so its border takes
  the accent, as a dialog border does. In the `skit` theme it keeps the picker color.
- `select_border` dims an idle select that a clipped row draws by hand, as `patch_idle_border`
  dims a whole one.

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
  `vt100::Cell` reports foreground, background, bold, dim, and inverse. It keeps only the last of
  bold and dim (`attrs.rs:44-52`), so a cell that the terminal received with both shows as one of
  them. `terminal_theme_titles_are_bold_and_never_dim` depends on that: a title that also carried
  dim shows as dim.
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
   `AppearanceBackend` are never called by a full-screen session, so no end-to-end test reaches
   them. Ratatui's `Terminal::clear` calls `clear_region`, not `Backend::clear`; only an inline
   viewport calls `append_lines`; and Ratatui never calls `window_size`. By the owner's ruling,
   an isolated test waited until the coverage gate asked for it. The Linux gate asked on
   2026-09-23 (`append_lines`, `clear`, `window_size`). The ways a forwarder can fail, written
   before the test:
   1. It calls a different method of the inner backend, such as `clear_region` for `clear`.
   2. It changes its argument, such as `append_lines(n)` that appends a different count.
   3. It returns its own value instead of the inner backend's, such as a fixed window size.
   4. It does nothing and returns `Ok`, which the trait's default `append_lines` does.
   `forwarders_reach_the_inner_backend` in `appearance.rs` checks each case against a Ratatui
   `TestBackend`, with and without `NO_COLOR`.
2. Role refactor (done 2026-09-23). Every production color in skit-tui now comes from a role
   function in `crates/skit-tui/src/theme.rs`. All 117 census files stayed byte-identical, and the
   old unit assertions pass unchanged. The census was extended first, to 39 screens; it reaches
   122 of 182 color sites (appendix B lists the rest).
   - The theme travels in a thread-local that `render_with_session` sets for one frame from
     `TuiSession`, with a guard that restores the previous value. Explicit passing would have
     changed about 67 functions and every unit test that renders directly. With the thread-local,
     a frame drawn without a session uses the `skit` theme, so the old assertions keep testing the
     `skit` look (owner option C, 2026-09-23). Phase 5 must add the census check that catches a
     site that bypasses the theme: no `#rrggbb`, no `idx:` of 16 or more, no foreground `idx:0`,
     `idx:7`, or `idx:15`, and no background without inverse, in every terminal-theme file.
   - Roles are split by their version 0.4 counterpart (see "Role counterparts"), so phase 3
     changes one role at a time.
3. Parity restorations (done 2026-09-23). Four end-to-end tests pinned version 0.4 first and
   failed: body text in the default foreground, hints as dim default text, scrollbars in
   `#4A413C`, and widget labels on the terminal background in the default foreground. Then the
   `text`, `hint`, and `scrollbar` roles and the unfocused radio, check box, and select text
   changed. A cell-by-cell comparison of the census showed only these changes: 4104 cells from
   bright white to default, 6212 cells from bright black to dim default, 3106 `NO_COLOR` cells
   that gained dim, and 414 scrollbar cells to `#4A413C`. No text and no background changed.
   Buttons with a fixed dark background keep white text. Seven old unit assertions that checked
   the port's white, bright black, or indigo scrollbar now check the version 0.4 value. One
   skit-tui PTY marker became one word, because Ratatui skips a default-style space.
4. `skit` theme color depth (done 2026-09-23). Three end-to-end tests failed first: no 24-bit
   color without `COLORTERM`, no color above 15 when `TERM` names no color count, and Rich's own
   256-color forms (accent 173, selection 254 on 52). skit-cli now decides the depth with Rich's
   rule (`tui_color_depth` in `crates/skit-cli/src/cli.rs`), and `theme.rs` maps each fixed color
   through `fixed()`, a table of Rich's precomputed forms. The census gained a `basic-term`
   environment (156 files). The `no-colorterm` files changed only from 24-bit colors to Rich's
   256-color indices; the `truecolor` and `no-color` files did not change. On Windows the depth is
   24-bit, because Textual turns off the legacy console and Rich then asks the console, which
   supports virtual terminal processing wherever skit's interface runs. `TEXTUAL_COLOR_SYSTEM`,
   a Textual override, is not ported.
5. `terminal` theme. Done on 2026-09-23 so far:
   - `skit config theme terminal|skit` (commit `a2a863bf`). The `config` help line keeps its
     version 0.4 wording, which a contract test pins, so the new key shows in `skit config`, in
     `--json`, and in shell completion instead.
   - Every census environment names its theme in `config.toml`. Four terminal-theme environments
     (dark, light, unknown background, `NO_COLOR`) bring the census to 312 files, and each
     terminal-theme frame passes the rule check: no 24-bit color, no index above 15, no black,
     white, or gray text, no background without reverse, and no hue on a cell that shows a letter
     or a digit.
   - `Theme::Terminal(accent)` in `theme.rs`. Widgets that take only colors get a post-pass:
     `patch_focus` reverses a focused button, check box, or radio option; `patch_idle_border`
     dims an idle select; `patch_plain` removes the colors that `Toast` and the removal dialog's
     buttons choose for themselves; `patch_chip_key` draws a footer key as a keycap without
     changing the chip width; `patch_border_title` and `border_title` keep a title on a colored
     border in the default foreground. `status_spans` and `status_line` color only a leading
     ✓ ✗ ⚠ → glyph; every locale keeps the glyph first. The add review's select and check boxes
     get their own roles, because the `skit` theme drew them in `ratatui-interact` defaults.
   - Background: skit-cli asks with `terminal-colorsaurus` 1.0.3 (1 s timeout) only when the
     theme is `terminal`, `NO_COLOR` is absent, and both standard streams are a terminal, before
     the interface claims the terminal. The PTY harness answers OSC 10, OSC 11, and DA1 in every
     color lane.
   - Preferences: a "Colors" section with a radio choice. It comes after every version 0.4
     control, because ported version 0.4 tests pin the focus order (Language and Editor are
     adjacent stops). A save writes `theme` only when it changed, so a clean save keeps the
     historical `config.toml` bytes. `PreferencesSaved` carries the new theme, `LibraryState`
     keeps it, and `render_with_session` applies it to the next frame with the session's color
     depth, background, and `NO_COLOR`. A session that started in the `skit` theme did not ask
     for the background, so a switch to `terminal` during the session has no accent hue until
     the next start.
   - The default switch (commit `7038e6f6`). `DEFAULT_THEME` in skit-store and `tui_theme` in
     skit-cli both give `terminal`, and `the_default_theme_is_terminal` and
     `the_terminal_theme_is_the_default` pin it. The switch sent the color question to every
     existing PTY test, and three failures showed that the answers share the input with the keys:
     - The question read the `Esc` that a test typed ahead. skit now asks only when no input is
       waiting (`crossterm::event::poll` with a zero timeout, in raw mode).
     - A late answer became keys: it opened the selected entry and typed `gb:ffff/…` into its
       form. After a timeout, skit now reads and drops input for 1 s
       (`a_late_color_answer_never_becomes_keys`, with a harness mode that answers 1.5 s late).
     - A slow remote link can deliver the answer after any timeout, so skit does not ask when
       `SSH_CONNECTION` or `SSH_TTY` is set (`a_remote_login_skips_the_color_question`).
     Harness waits that looked for raw bytes now wait on the replayed screen, because Ratatui
     skips a cell whose style did not change. The full skit-cli suite (1902 tests) passed on
     macOS, and the full workspace (5468 tests) passed on Linux.
   - Windows (2026-09-23). No Windows host is available, so the question on Windows is gated
     before it is tested. The ways it can fail, written before the code:
     1. A console that answers neither OSC 11 nor DA1 makes skit wait 1 s and then drop input
        for 1 s. The first keys are lost. The `run_in_pty` tests in `terminal_pty.rs` send `q`
        after a short quiet time, so they would hang on the CI Windows job.
     2. A console that answers DA1 itself and also passes the question on gets a second answer
        late. On Windows, the console turns an escape sequence it does not know into key events,
        so the answer can reach the interface as keys.
     3. Windows Terminal before 1.22 answers only DA1. skit gets an unknown background at once.
        This is safe.
     4. A console that is not Windows Terminal inherits `WT_SESSION` from a parent process. The
        question then goes to a console of case 1 or 2.
     5. The gate reaches Unix too. The census `terminal-dark` and `terminal-light` files then lose
        their accent, and the census fails.
     The rule: on Windows, skit asks only when `WT_SESSION` is set, because
     `terminal-colorsaurus` supports Windows Terminal 1.22 and later. Case 4 stays open.
     `a_windows_console_outside_windows_terminal_gets_no_color_question` pins cases 1 and 2 on
     the CI Windows job. The Windows accent in Windows Terminal is not verified on a real host.
   The reverse selection under `NO_COLOR` is tested here, as an addition. When `Appearance` gains
   a field, `with_no_color` must build `Self { no_color, ..self }`, or it drops the theme.
6. Documentation, records, and demo assets. Done on 2026-09-23 so far:
   - The documentation site: the `theme` key and its notes in `configuration`, and `NO_COLOR`,
     `COLORTERM`, the `TERM` suffix, and `SSH_CONNECTION` / `SSH_TTY` in `environment`, in
     English, Simplified Chinese, and Traditional Chinese.
   - `docs/behavior-changes.md`: the default theme, the `theme` setting and its Preferences
     section, the color question with its input rules, and the reverse selection under
     `NO_COLOR`. No item went to `docs/parity-backlog.md`: every version 0.4 gap that this design
     found was restored in phases 1, 3, and 4.

At the end of each wave, the Linux prompt reruns the census on Linux and diffs it against the
committed files, and runs the walker corpus.

## Appendix A: role sites

Generated from the source on 2026-09-23 (phase 2). Each row lists the functions that call the
role.

| Role | Calls | Functions |
| --- | --- | --- |
| `action_button_style` | 2 | screens/preferences.rs `render_agent_skill_picker`, screens/preferences.rs `render_control` |
| `border` | 3 | session.rs `render_header`, session.rs `render_textarea_band` ×2 |
| `border_color` | 1 | session.rs `render_line_input_band` |
| `caret` | 1 | session.rs `render_textarea_band` |
| `checkbox_style` | 2 | screens/settings.rs `draw_control`, session.rs `render_run_control` |
| `dialog_button_style` | 2 | screens/modal.rs `discard_changes`, screens/modal.rs `render` |
| `dialog_footer_chip_style` | 1 | footer.rs `dialog` |
| `emphasis` | 1 | screens/library.rs `detail_lines` |
| `focused_radio_style` | 1 | session.rs `render_radio_option` |
| `footer_chip_style` | 2 | footer.rs `default`, footer.rs `render_with_decoration` |
| `footer_indicator` | 1 | footer.rs `render` |
| `heading` | 2 | screens/preferences.rs `render`, screens/settings.rs `render_settings` |
| `hint` | 20 | screens/management.rs `render_summary`, screens/management.rs `render`, screens/preferences.rs `render_agent_skill_picker` ×2, screens/preferences.rs `render_control`, screens/preferences.rs `render`, screens/preferences.rs `runner_row_spans` ×2, screens/settings.rs `draw_control`, screens/settings.rs `render_read_only` ×2, screens/settings.rs `render_settings`, session.rs `run_field_label` ×2, session.rs `run_field_notes` ×5, session.rs `run_layout` |
| `key_hint` | 2 | screens/settings.rs `render_settings` ×2 |
| `list_picker_style` | 4 | screens/management.rs `render_issues`, screens/preferences.rs `render_agent_skill_picker`, screens/run_modal.rs `render_environment`, screens/run_modal.rs `render_token_menu` |
| `marker` | 5 | screens/add.rs `render_row`, screens/preferences.rs `paint_focus_marker`, screens/settings.rs `render_options` ×2, session.rs `render_radio_option` |
| `muted` | 26 | screens/add.rs `hint`, screens/add.rs `render_footer` ×2, screens/add.rs `review_rows` ×9, screens/add.rs `source_rows` ×2, screens/library.rs `append_state_lines`, screens/library.rs `append_storage_mode`, screens/library.rs `detail_lines` ×3, screens/modal.rs `render`, screens/picker.rs `render_file_picker` ×3, screens/picker.rs `render_picker_footer_items` ×2, screens/picker.rs `render_prompt_candidate_picker` |
| `notice` | 2 | screens/add.rs `build_rows`, screens/add.rs `kind_rows` |
| `panel Add` | 1 | screens/add.rs `render_add` |
| `panel Detail` | 2 | screens/library.rs `render_detail` ×2 |
| `panel Dialog` | 6 | screens/management.rs `render`, screens/modal.rs `discard_changes`, screens/modal.rs `render` ×3, screens/run_modal.rs `modal_block` |
| `panel Form` | 1 | session.rs `render_form` |
| `panel Health` | 1 | screens/management.rs `render` |
| `panel Library` | 1 | screens/library.rs `render` |
| `panel Picker` | 3 | screens/picker.rs `render_file_picker`, screens/picker.rs `render_prompt_candidate_picker`, screens/preferences.rs `render_agent_skill_picker` |
| `panel Preferences` | 1 | screens/preferences.rs `render` |
| `panel Run` | 2 | session.rs `render_run`, session.rs `run_scrollbar_style` |
| `panel Settings` | 2 | screens/settings.rs `render_settings`, screens/settings.rs `settings_scrollbar_style` |
| `panel_border` | 2 | screens/modal.rs `render`, screens/run_modal.rs `modal_block` |
| `panel_color` | 2 | screens/modal.rs `render` ×2 |
| `radio_style` | 1 | session.rs `render_radio_option` |
| `required` | 1 | session.rs `run_field_label` |
| `run_chip_style` | 1 | session.rs `render_run_chips` |
| `scrollbar` | 2 | screens/settings.rs `settings_scrollbar_style`, session.rs `run_scrollbar_style` |
| `select_style` | 5 | screens/preferences.rs `render_control` ×2, screens/preferences.rs `render_open_dropdowns`, session.rs `render_open_dropdowns`, session.rs `render_run_control` |
| `selection` | 7 | screens/add.rs `render_row`, screens/library.rs `render`, screens/picker.rs `render_file_picker` ×2, screens/settings.rs `render_options`, session.rs `render_textarea_band` ×2 |
| `status` | 29 | screens/add.rs `build_rows`, screens/add.rs `confirm_draft_delete_rows`, screens/add.rs `review_rows` ×3, screens/library.rs `append_state_lines` ×3, screens/library.rs `last_run_line`, screens/management.rs `render_summary` ×9, screens/management.rs `render` ×2, screens/picker.rs `render_file_picker`, screens/preferences.rs `runner_row_spans`, screens/run_modal.rs `render_file`, screens/run_modal.rs `render_preset`, session.rs `run_field_notes` ×3, session.rs `run_layout` ×2 |
| `suggestion` | 1 | session.rs `render_line_input_band` |
| `table_header` | 1 | screens/library.rs `render` |
| `text` | 13 | screens/preferences.rs `render_radio_band`, screens/preferences.rs `runner_row_spans`, screens/settings.rs `render_options` ×2, screens/settings.rs `render_read_only`, screens/settings.rs `render_settings` ×2, session.rs `render_flat_search_input`, session.rs `render_line_input_band` ×2, session.rs `render_textarea_band`, session.rs `run_field_label`, session.rs `run_layout` |

## Appendix B: color sites the census does not reach

Measured with `cargo llvm-cov` over the census on 2026-09-23, before the role refactor. Each group
needs a state the census fixture does not create.

- `add.rs` `review_rows` (11): detected-parameter kinds other than a shell constant (argparse,
  environment reads, unsupported shapes).
- `add.rs` `build_rows`, `source_rows`, `kind_rows`, `hint`, `confirm_draft_delete_rows`,
  `render_footer`: an add notice, an ambiguous kind, a draft to delete, and a scrolled footer.
- `library.rs` `append_state_lines` (3), `detail_lines`: a missing target, a drifted script, and
  a reference-mode entry (its path is a host path).
- `management.rs` `render_summary` (3), `render`: uv found (depends on the host), invalid runner
  rows, and a runner editor error.
- `modal.rs` `render` (4): the compact removal dialog of a very small terminal.
- `picker.rs` `render_file_picker` (5), `render_picker_footer_items`,
  `render_prompt_candidate_picker` (2): a multi-select picker, an I/O error, and the prompt
  variable picker.
- `preferences.rs` `render_agent_skill_picker`, `runner_row_spans`: an install hint and an
  invalid runner row.
- `run_modal.rs` `render_file`, `render_preset`: a file error and a preset name error.
- `settings.rs` `render_options` (2), `render_read_only` (3), `render_settings` (4): an open
  option list, read-only fields, and the `Ctrl+N` and `Ctrl+O` rows.
- `session.rs` `run_field_notes` (6), `run_layout` (2), `render_header`: expansion, token, and
  glob notes; drift and degradation notices; and the compact header.
- `footer.rs` `render`: the scroll arrow of a dialog action row.

The role refactor kept these sites on the same values, and the old unit assertions still cover
many of them.
