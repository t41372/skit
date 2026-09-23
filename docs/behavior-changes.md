# Deliberate behavior changes in version 0.5

Version 0.5 must be a strict superset of the behavioral oracle (`origin/main@206f9ef`, Python
`0.4.1.dev0`). This file records each place where version 0.5 does something version 0.4 does not,
so a reviewer can tell a deliberate addition from an accidental difference. `docs/parity-backlog.md`
records the opposite: a version 0.4 behavior version 0.5 does not have yet.

Add an item here when the change is user-visible. Name the product rule or the oracle line that
makes the change correct. An addition is permitted; a removal is not.

## The settings screen offers the environment-default rewrite

`skit-ui` puts a `source:normalize` control in the parameter section of a stored shell copy. It ticks
one or more constants, and the save rewrites each `NAME=value` as `NAME="${NAME:-value}"` in the
stored copy.

Version 0.4 has no such control. It offers the rewrite only as `skit params <entry> --normalize NAME`
(`src/skit/cli.py:4113-4116`), and its terminal interface points the user at that command in a hint
(`src/skit/cli.py:4014`).

Product rule 3 makes the hint the defect: "Keep the interface discoverable. A user must not need to
remember commands, keys, or script arguments." A sentence that names a command line is exactly the
case that rule refuses.

Two properties keep the control safe:

- It is strictly opt-in. The option set opens with nothing ticked, and a save that touches no tick
  carries no `source:normalize` value at all. `AGENTS.md` calls `--normalize` "the only opt-in
  semantic edit to a stored script", so a default-on box would make a rename rewrite a script.
  `every_tick_to_act_offer_opens_empty_and_a_save_that_touches_none_carries_none` pins this.
- It never offers a constant that already reads `${NAME:-value}`. That form is the result of this
  rewrite, and the normalizer refuses a value that names itself
  (`crates/skit-language/src/semantic/shell.rs:747-751`), so offering it again would produce only a
  refusal.

## The resync is a control, not only a chord

Version 0.4 binds `Ctrl+R` to `action_resync`, which reads the script's parameter definitions again
and rebuilds the screen (`src/skit/tui_settings.py:269`, `:908-926`). There is no click target and no
way to take the request back.

Version 0.5 gives the same chord, and the chord reaches a visible checkbox in the parameter section.
Product rule 2 requires it: "Keep each TUI action available by keyboard and mouse." The checkbox is
also reversible, so a person who presses the chord by mistake can untick it.

The request now applies when the save runs, rather than immediately. The pre-save report version 0.4
prints is not built yet; it is recorded in `docs/parity-backlog.md`.

## The preset deep link moves the keyboard, not only the viewport

Version 0.4's Library gives `s` to `action_settings(section="presets")` (`src/skit/tui.py:991-992`),
and the settings screen scrolls its body to that section on mount
(`src/skit/tui_settings.py:876-882`). The keyboard stays where the screen put it, which is the name
box, so the first key press after `s` types into the name.

Version 0.5 also puts the keyboard on the first preset. Product rule 2 asks every action to have a
keyboard path, and a person who pressed `s` came to act on a preset: `Space` deletes the first one
immediately, rather than typing a space into an unrelated field.

A section with nothing to edit keeps the anchor instead, so an entry with no presets still lands on
the sentence that says where presets come from. The anchor is released by the first keyboard move,
after which the viewport follows the focus as it does on every other screen.

## Entry settings is one typed screen, and two form surfaces were deleted

The composition root now opens `Screen::Settings` for both `HostRequest::Settings` and
`HostRequest::Presets`. Two interim surfaces were deleted with it: the flat `tui_settings_form`,
which listed every axis as a text box, and `tui_presets_form` with its `FormPurpose::Presets`.
Neither existed in version 0.4, so this removes nothing the oracle has. It does remove two keys a
machine path could previously send from the terminal frontend, and each deletion is argued here
rather than assumed — "the capability is covered elsewhere" is the claim that has to be checked.

`FormPurpose::Presets` carried a `name` and an `action` of `save` or `delete`.

- **Save** duplicated the run form. Version 0.4 creates a preset with `Ctrl+S` inside the run form
  and nowhere else, which is what its own empty-state sentence tells the user
  (`src/skit/tui_settings.py:804-808`). That path is wired end to end in version 0.5:
  `UiCommand::SavePreset` opens `ModalState::RunPresetName`, which produces
  `UiEffect::SaveRunPreset`, which the host answers with `FormStateService::save_preset` and a
  refreshed picker.
- **Delete** is now unticking the preset in entry settings, which is version 0.4's own affordance
  (`:809-818`, `:1114-1120`).

`source:unmanage` and `parameter:remove` are no longer produced by any screen.

- Both are the keep toggle on a parameter row. Version 0.4 has no separate control for either: an
  unticked `ParamRow` leaves the block and an unticked `DeclParamRow` leaves the schema
  (`src/skit/tui_settings.py:115-117`, `:1061`). A row carries its own name, so the toggle names
  what it removes.
- Both keys are still read by `tui_submit_settings`, so `skit params` and an agent that posts a
  submission keep their path. A contract test submits each key on its own to prove it.

The completeness check for this deletion was an inventory, not the compiler. Every key the flat form
produced was listed and matched against what produces it now. Fourteen moved to a typed control, two
are the toggles above, and two more — `preset:{name}` and `parameter:{name}:{axis}` — are new. A
map-lookup key hides a dead producer behind a live function call, so `cargo clippy` reports nothing
for a key nobody sends any more; it caught only `settings_parameter_fields`.

## The add-a-parameter box takes more than one name

Version 0.4 reads one name from its add box and makes one declaration
(`src/skit/tui_settings.py:719`, `:747-749`). Version 0.5 splits the same box on commas and spaces,
so one save can add several parameters. The single-name case behaves the same way.

## A PowerShell `[bool]` parameter is a native checkbox

Version 0.4 reads a PowerShell `param()` block through the script's own parser and maps each static
type onto the form's type axis. Its `_STATIC_TYPES` map
(`src/skit/langs/powershell/cli_reader.py:63-69`) lists `System.String`, `System.Int32`,
`System.Int64`, `System.Double`, and `System.Single`. It omits `System.Boolean`, so a `[bool]`
parameter falls to `_apply_static_type` (`:253-260`), which degrades it to a free-text field.

Version 0.5 maps `[bool]` and `[boolean]` to `ParameterType::Bool`, a native checkbox, and carries a
readable `$true` / `$false` default onto it. This makes a Boolean typed parameter a correctly typed
control instead of free text, which is how skit already reflects a Boolean in every other language
(Python `store_true`, an argparse bool, and so on), so it removes an inconsistency rather than a
version 0.4 feature. A `[switch]` was already a native `store_true` toggle in both versions; this
extends the same treatment to the explicit `[bool]` spelling.

The change is an addition, not a removal: the free-text field the oracle produced could hold only a
Boolean value anyway, and the checkbox delivers the same `-On`/`-On:$false` flag with a discoverable
control. `test_bool_default_is_carried` in `crates/skit-language/tests/port_test_powershell.rs`
pins the kept behavior (`default == Some(Bool(true))`, not degraded).

## The interface follows the terminal's colors by default

Version 0.4 has one fixed palette, `CLAUDE_THEME` (`src/skit/theme.py:46-58`), with a terracotta
accent (`:20`), near-white titles, and a warm selection bar. No setting changes it.

Version 0.5 adds the `theme` setting with two values. The owner picked `terminal` as the default on
2026-09-23 (`docs/design/terminal-palette.md`).

- `terminal` draws text in the terminal's default foreground on its default background. Bold,
  dim, and reverse video show the state. One accent hue marks focused borders, selection markers,
  and footer keys: cyan on a dark background, magenta on a light background, and no hue when the
  background is unknown. A status line colors only its ✓ ✗ ⚠ → glyph. A footer key is a reversed
  keycap. An idle button shows brackets, as in `[Discard]`, and a focused button is reversed.
- `skit` keeps the version 0.4 palette, with the parity restorations of the design, so nothing is
  removed. `skit config theme skit` or the Preferences "Colors" section gives it back.

A user who does not change the setting sees a different look after the upgrade. This is the
deliberate change. Every state that the `skit` theme shows with a color, the `terminal` theme shows
with an attribute. The census in `crates/skit-cli/tests/tui_palette_census.rs` checks every
terminal-theme frame: no fixed color, no background without reverse video, and no hue on a letter
or a digit.

A downgrade is safe. Version 0.4 reads `config.toml` as a whole and keeps a key it does not know
(`src/skit/config.py:91-103`).

## `skit config theme` is a new setting

Version 0.4 refuses `skit config theme` with "Unknown setting" and exit code 2
(`src/skit/cli.py:5511-5515`). Version 0.5 reads and writes the key with `--json`, deterministic
exit codes, and shell completion, as product rule 4 asks. The `skit config` listing and its
`--json` object have one more key. An unknown value exits with code 2 and does not change
`config.toml`. `crates/skit-cli/tests/theme_config.rs` pins this.

The Preferences screen gets a "Colors" section with the same choice, as product rule 4 asks in the
other direction. The section comes after every version 0.4 control, so Tab visits the version 0.4
controls in the version 0.4 order. Shift+Tab from the first control now reaches the palette choice
first, before the last mirror control. A save writes `theme` only when the value changed, so a save without a theme change
keeps the version 0.4 `config.toml` bytes.

## skit asks the terminal for its background color

Textual 8.2.8, the version 0.4 interface library, does not ask for the background color. It
sends no OSC 10 or OSC 11 sequence: its only OSC sequences are 52 (clipboard) and 22 (pointer
shape).

Version 0.5 must know the background to pick the accent of the `terminal` theme. Before the
interface starts, skit sends the OSC 10, OSC 11, and device-attributes questions through
`terminal-colorsaurus` and waits up to 1 second. It asks only when all of these are true:

- The theme is `terminal`, and `NO_COLOR` is not set.
- Standard input and standard output are both a terminal.
- `SSH_CONNECTION` and `SSH_TTY` are not set. A slow link can deliver the answer after any timeout,
  and a late answer arrives on the same input as the keys.
- On Windows, `WT_SESSION` is set. `terminal-colorsaurus` supports Windows Terminal 1.22 and later.
  Another Windows console can answer nothing, or answer late and turn the answer into keys.
- No input is waiting. Keys that the user typed before the question stay for the interface.

A terminal that answers only the device-attributes question gives an unknown background at once,
with no wait. When the question times out, skit reads and drops input for 1 more second, so a late
answer cannot reach the interface as keys. A key typed in that second is lost. Without this rule, a
late `ESC ] 11 ; rgb:…` answer opened an entry and typed the rest of the answer into its form; the
test `a_late_color_answer_never_becomes_keys` pins the fix.

## `NO_COLOR` in the terminal theme reverses the selection

Under `NO_COLOR`, version 0.4 removes every color (Textual `NoColor`). The selected row keeps only
bold, and every border has the default foreground. The `skit` theme keeps that result.

The `terminal` theme shows the state with attributes before any filter, so under `NO_COLOR` its
selected row is bold and reversed, and an idle border is dim. With the new default, a `NO_COLOR` user
sees this look. It adds a visible state and removes none.
