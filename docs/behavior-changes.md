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

Ticking the checkbox reads the definitions at once, as version 0.4 does. The host runs the resync
that the save runs and writes nothing. The parameter rows show the result, and the report sits
under the checkbox: the analyzer warnings, or `Everything still matches the script.`
(`src/skit/tui_settings.py:917-925`). The save keeps the result. It runs the resync again on the
script as it is at that time, so a script that changed in the meantime cannot be overwritten with an
old read. Unticking takes the request back: the rows the screen opened with return, and the report
goes away.

Version 0.4 rebuilds the whole screen after `Ctrl+R` (`:926`), and every unsaved edit in every
section is lost. Version 0.5 rebuilds only the parameter rows. An edit in another section stays,
and an edit on a row that the resync keeps moves to the rebuilt row. Version 0.4 also prints the
report at the end of the section, which is often below the fold. Version 0.5 scrolls the report
into view together with its checkbox.

## The full variable picker has a footer chip

Version 0.4 opens the complete list of a prompt's detected variables with `Ctrl+O`, and the only
place that names the key is a chip under the capped inline list (`src/skit/tui_settings.py:702-717`).
Its binding is hidden from the footer (`:271-276`). The capped list is long, so the chip is below
the fold on a normal terminal.

Version 0.5 also shows the chip in the footer, while the inline list is capped and variable
insertion is on, which is exactly when the key does something. Product rule 3 requires it: "A user
must not need to remember commands, keys, or script arguments." A click on the footer chip opens
the same picker as the key and the inline chip.

## A placeholder added by name is secret when its name says so

Version 0.4 makes two kinds of declaration for a prompt's placeholder. A ticked candidate is
required, and secret when its name says so (`src/skit/tui_settings.py:1122-1139`). A name typed in
the add-a-parameter box is required and never secret (`:721-730`).

Version 0.5 makes the first kind on both paths, so one placeholder gets one declaration however a
person brings it under management. A name such as `api_key` that the box adds is secret, so its
value is not remembered. The person can untick the secret checkbox before the save.

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
  background is unknown. Success text is green and warning text is yellow, as in version 0.4. An
  error colors only its ✗ glyph. A footer key is a reversed keycap. An idle button shows
  brackets, as in `[Discard]`, and a focused button is reversed.
- `skit` keeps the version 0.4 palette, with the parity restorations of the design, so nothing is
  removed. `skit config theme skit` or the Preferences "Colors" section gives it back.

A user who does not change the setting sees a different look after the upgrade. This is the
deliberate change. Every state that the `skit` theme shows with a color, the `terminal` theme shows
with an attribute. The census in `crates/skit-cli/tests/tui_palette_census.rs` checks every
terminal-theme frame: no fixed color, no background without reverse video, and no hue on a letter
or a digit except the green and yellow of status text.

A downgrade is safe. Version 0.4 reads `config.toml` as a whole and keeps a key it does not know
(`src/skit/config.py:91-103`).

## `skit config theme` and `skit config accent` are new settings

Version 0.4 refuses `skit config theme` and `skit config accent` with "Unknown setting" and exit
code 2 (`src/skit/cli.py:5511-5515`). Version 0.5 reads and writes both keys with `--json`,
deterministic exit codes, and shell completion, as product rule 4 asks. The `skit config` listing
and its `--json` object have two more keys. An unknown value exits with code 2 and does not change
`config.toml`. `crates/skit-cli/tests/theme_config.rs` pins this.

- `theme` is `terminal` (default) or `skit`.
- `accent` picks the one hue of the terminal theme: `auto` (default: cyan on a dark background,
  magenta on a light one, none when the background is unknown), `none`, or one of `red`, `green`,
  `yellow`, `blue`, `magenta`, `cyan`, and their `bright-` forms. A color name picks that color of
  the terminal's own palette. Black, white, and gray are not choices, because the terminal theme
  never draws a mark in them. The `skit` theme ignores the setting.

The Preferences screen gets a "Colors" section with the same two choices, as product rule 4 asks
in the other direction. The section comes after every version 0.4 control, so Tab visits the
version 0.4 controls in the version 0.4 order. Shift+Tab from the first control now reaches the
accent choice first, then the palette choice, before the last mirror control. A save writes
`theme` or `accent` only when the value changed, so a save without a change keeps the version 0.4
`config.toml` bytes.

## skit asks the terminal for its background color

Textual 8.2.8, the version 0.4 interface library, does not ask for the background color. It
sends no OSC 10 or OSC 11 sequence: its only OSC sequences are 52 (clipboard) and 22 (pointer
shape).

Version 0.5 must know the background to pick the accent of the `terminal` theme. Before the
interface starts, skit sends the OSC 10, OSC 11, and device-attributes questions through
`terminal-colorsaurus` and waits up to 1 second. It asks only when all of these are true:

- The theme is `terminal`, the accent is `auto`, and `NO_COLOR` is not set. Any other accent
  names its hue, so skit has no question to ask.
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

## Search lists the best match first

Version 0.4 filters the Library with a case-insensitive subsequence match and keeps the matches in
recency order (`src/skit/tui.py:85-95`, `:398-405`). Enter in the search box runs the top row
(`:614-619`). A weak match in a description can therefore stand above the entry that the query
names. For example, `hello` listed `review` first, because the description of `review` holds the
letters of `hello` in order, and Enter opened `review`.

Version 0.5 keeps the same filter and changes the order. An entry whose name starts with the query
comes first, then the higher match score. Recency decides between equal matches. Each change to the
query selects the first row, as the table rebuild of version 0.4 does, so Enter opens the best
match. `search_lists_the_best_match_first_and_enter_opens_it` and the `search-ranked` census screen
pin the order.

## Rename is a small dialog, and Enter submits it

Version 0.4 renames an entry only through the Name box of its settings screen. Version 0.5 adds
`F2`, which opened a whole screen for one box, and Enter in that box only moved the focus. Now
Rename is a small dialog in the middle of the screen. Enter renames and Esc cancels, and the footer
shows both keys. The dialog repeats the note that Entry settings shows beside the same box.

The rule is general for a generic form: Enter submits from a single-line field, as Enter runs the
launch form. A multi-line field keeps Enter for a new line, and `Ctrl+S` submits from there. A form
with one field shows no field keys in its footer. `rename_dialog_renames_the_entry_on_enter`
pins the dialog on the real binary.

## Add keeps a typed Name and Description for a path

The Add screen of version 0.4 takes the path first, and a Name or a Description typed in the
command template boxes is not used for the path (`src/skit/tui_add.py:455-463`). Nothing tells the
person that the values are lost.

Version 0.5 puts the two boxes in the template part of the screen, with the labels
"Name for the command" and "Description (optional)", as version 0.4 does with its placeholders.
When a path or a draft opens the review anyway, a typed value fills the Name or the Description of
the review. The person sees it there and can change it before the entry is added.
`typed_command_name_and_description_prefill_the_review_of_a_path` and
`a_typed_name_and_description_reach_the_entry_of_a_path` pin this.

## A second pick of a kept draft continues with it

Version 0.4 resumes a kept draft with one pick in its list (`src/skit/tui_add.py:332-336`).
Version 0.5 puts a picked draft in the path box first, so that `Delete draft…` can act on the row
that the mouse picked. A second pick of the same draft now continues with it, and the footer
names that key `Enter Continue`. Before, the person had to find the Continue button.

## A new prompt draft takes its name from its first words

Version 0.4 names an entry that comes from a new draft after the temporary file, for example
`skit-new-91G1LH`, and shows that name in the Library and in the review title. It also suggests
the starter line `# New prompt` as the description of a new prompt.

Version 0.5 skips the starter line that skit writes. The description comes from the first line that
the person wrote, and the name comes from the first four words of that line. The name drops the
braces of `{{name}}` placeholders and does not end with a sentence mark. The store makes the slug
from this name with its usual rules. A name that another entry already has is refused when the
person adds the entry, as for every other name, and the review keeps the draft so that the person
can change the name. A typed name wins, and a prompt file that is not a draft keeps its file name.
`a_new_prompt_draft_is_named_from_its_first_words` pins this on the real binary.

## Each status line reports its outcome, and a receipt clears itself

Version 0.4 writes a status into one line under the Library and keeps it until the next search or
reload replaces it (`src/skit/tui.py:430-441`). A few receipts are Textual toasts with their own
timeout instead, for example `Preset "triple" saved.`. A status has no outcome of its own, so an
error and a receipt look the same.

Version 0.5 gives every status one of four kinds in `skit-ui` (`StatusKind`): information,
success, warning, and error. The footer draws a success with `✓`, a warning with `⚠`, and an error
with `✗`, in the colors that the theme gives status text. Information has no glyph. A success or
information line clears itself after 4 seconds, and the footer shows its usual line again, for
example the entry count. A warning or an error stays until the next key press, paste, or click,
because the user must read it to act. A status that is wider than the footer wraps onto a second
row. Product rule 3 asks for this: a person who looks away must still see that a run failed, and a
receipt must not hide the entry count for the rest of the session.

These receipts are new. Version 0.4 is silent at each of them:

- `Ctrl+R` in the Library shows `✓ Library reloaded (N entries)`.
- A choice in the preset picker shows `✓ Preset applied.`, or `✓ Last values restored.` for the
  `↩ last values` row (`src/skit/tui_form.py:761-779` applies the values without a word).

## A run from the interface reports its outcome on both sides of the screen

Version 0.4 prints `── Run <name> ──` before a run from the Library (`src/skit/tui.py:829`,
`:1068-1072`), and a session that the run ends returns the exit code of the run
(`src/skit/tui.py:1067-1103`). Version 0.5 keeps both. Two lines differ:

- A run that ends the session (`after_run = "exit"`, the default) prints one result line after
  the output of the script: `✓ <name> finished`, or `✗ <name> exited with code N`. Version 0.4
  prints nothing there (`src/skit/tui.py:1067-1103`). Its `skit run` prints
  `Run exited with code N` (`src/skit/cli.py:3244-3248`), so a person who started the run from the
  interface now sees the same fact. The owner decided on 2026-11-08 that both outcomes print a
  line, so the shell never shows the output of a script with no word about how the run ended.
  `a_successful_run_that_ends_the_session_prints_its_result_line_and_exits_with_zero` pins this on
  the real binary.
- A run that returns to the Library (`after_run = "stay"`) reports `✓ <name> finished` or
  `✗ <name> exited with code N` as a status. Version 0.4 says `Last: <name> ✓ finished` or
  `Last: <name> ✗ failed (code N)` (`src/skit/tui.py:857-862`). The glyph now comes from the
  status kind, so the line starts with it.

A run that cannot start because a program is not on PATH says so with the next step:
`✗ pwsh is not on PATH. Install it, or set the interpreter in Entry settings.` A prompt names
another agent as the step, and a command template says to install the program. Version 0.4 refuses
the same run before its launch menu opens, with an error that names only the interpreter
(`src/skit/langs/launch.py:466`).

## Confirmation dialogs have a focus ring

Version 0.4 draws the actions of the Remove and Discard dialogs as footer chips, `y Remove` and
`Esc Keep`, and `y Discard` and `Esc Keep editing` (`src/skit/tui.py:160-165`,
`src/skit/tui_settings.py:58-63`). The chips are links: no key moves between them, and Enter does
nothing.

Version 0.5 draws the same chips and adds a focus ring. Tab and Shift+Tab move it, and Enter
activates the chip under it. Product rule 2 asks for a keyboard path to each visible target. The
ring opens on the answer that keeps the work, `Keep` or `Keep editing`, every time a dialog opens,
so a reflexive Enter can never remove an entry or throw edits away.

## Footer chips name their navigation keys and both quit keys

Version 0.4 shows the field navigation chips without words, `Tab/↓` and `Shift+Tab/↑`
(`src/skit/tui_footer.py:82-94`), and its runner editor names them. Every footer in version 0.5 now
names them `Next field` and `Previous field`, so one chip reads the same on every screen.

The Library footer shows `q/Esc Quit`, and Help lists `q / Esc / Ctrl+C Ctrl+C`. Version 0.4 shows
`q Quit` in the footer and `Ctrl+C Ctrl+C / Esc` in Help (`src/skit/tui.py:210`), so each surface
left out a key that the other one named.

## Chips and buttons follow the pointer

Version 0.4 tints a footer chip under the pointer. Version 0.5 makes a chip or a button under the
pointer bold and underlined, and inverts the chip that a primary press holds until the release.
This covers the footers, the dialogs, Preferences, the buttons and footer chips of Add, and the
footer chips of the file picker and the prompt variable picker. Attributes carry both looks, so
they show in both themes and under `NO_COLOR`.

## A refused value marks its field

Version 0.4 shows a refused launch-menu value as red text under the field. Version 0.5 starts that
line with `✗` and draws the border of the field in red and bold, so the field shows before the text
is read.

## The screen that asks for parameters is the launch menu

Version 0.4 calls the screen where a person fills the parameters and runs the entry a form, for
example "the run form asks for them", "Interactive form", and "Form label:". Its README already
calls the same screen the launch menu. The owner decided on 2026-11-08 that the product uses one
name: every English, Simplified Chinese (启动菜单), and Traditional Chinese (啟動選單) text in the
interface, the CLI help, the messages, the Agent Skill, and the documentation now says launch menu.
The parameter declarations that the script's block keeps are "parameter definitions". The machine
contracts stay: the `form` setting with its values `tui` and `plain`, the JSON keys, and the flag
names. `every_catalog_row_calls_the_parameter_screen_the_launch_menu` pins the catalog.

## The footer keeps its border

Version 0.4 docks a bare `KeysBar` with no border under every screen
(`src/skit/tui_footer.py:97-108`, and visible in every shipped frame under `docs/assets/`).
Version 0.5 draws the footer keys inside a rounded box. The owner decided on 2026-11-08 to keep the
border, so the density of every screen stays as it is.

The trade-off: the border costs two rows of body on every terminal, so the launch menu can need a
scroll to show its argument tail, which the version 0.4 demo frame shows without one. On the
recorded demo terminal, Entry settings shows its parameter section and one row past it, and version
0.4 shows the same rows and two more. In return, the box sets the keys apart from the body as one
group.

## Every text box edits the same way

Version 0.4 uses the Textual `Input` in every box, and its Description box has one line. Version 0.5
has one editor for every single-line box and for the multi-line Description and Help boxes. The
keys, the selection, the clipboard, the undo history, and the paste rules are the same in each box.
Help lists the keys in an "Editing text" section.

These parts are new in version 0.5:

- `Ctrl+Z` undoes and `Ctrl+Y` redoes in every box. One undo takes back one typed word, one paste,
  or one delete command. Version 0.4 has no undo in a box.
- `Alt+D`, `Ctrl+Delete`, and `Alt+Delete` delete the word after the cursor. `Alt+←` and `Alt+→`
  move by word, as `Ctrl+←`, `Ctrl+→`, `Alt+B`, and `Alt+F` do. In the Add screen of version 0.4,
  `Alt+←` closes the screen.
- A double click selects the word under the pointer.
- `Ctrl+C` copies a selection to the session clipboard and asks the terminal to put it on the
  system clipboard (OSC 52). Without a selection, `Ctrl+C` keeps the two-press quit. `Ctrl+X` cuts
  the selection, and `Ctrl+V` pastes the session clipboard. A paste from the terminal itself
  arrives as a bracketed paste.
- In a local session, a copy also goes to the clipboard program of the platform: `pbcopy` on
  macOS, and `wl-copy`, `xclip`, or `xsel` on Linux. Terminal.app ignores OSC 52, and iTerm2 and
  tmux block it by default, so a local copy still reaches the system clipboard there. An SSH
  session uses OSC 52 only, because the clipboard of the remote machine is not the clipboard of the
  person. Windows Terminal takes OSC 52, so Windows uses no program. The program runs in the
  background, and a missing program is not an error. A copy shows no receipt: skit cannot know
  whether the terminal or the program put the text on the system clipboard, and the selection
  stays on the screen.
- A multi-line box keeps every line of a paste, with each CRLF or CR changed to LF. A single-line box
  keeps the first line, as version 0.4 does, and a Tab in that line stays in the value.
- A word follows the Unicode word boundaries (UAX #29), so a slash separates two words and
  `two.py` is one word.

skit turns on bracketed paste with the other screen modes and turns it off while a run has the
terminal. crossterm on Windows reads console input records and reports no paste event, and the
legacy Windows console refuses the mode. skit does not ask for the mode on Windows, so there a
terminal types a paste as keys. skit reads the keys that arrive together, each one 5 ms or less
after the key before it, as one burst. A burst that holds a typed character and also a line break
or a Tab is a paste. It goes to the focused box with the paste rules of that box, so it never runs
the launch menu and never moves the focus. A key that a person presses alone stays a key, and
Enter runs the launch menu at once.

These limits remain on Windows:

- A paste of only line breaks or Tabs stays as keys, because a held Enter or Tab makes the same
  burst. A pasted line break with no other text acts as Enter.
- A paste with no line break and no Tab stays as typed keys. In a box, the text is the same as
  after a paste. In the Library, each character acts as its key.
- A long paste that the terminal sends in parts, with a pause of more than 5 ms between two
  parts, is more than one burst. A line break at the end of a part can act as Enter.
- When skit is busy and a person types a character and then Enter before skit reads them, the
  two keys arrive together, and skit takes them as a paste: the launch menu does not run. Press
  Enter again.

`a_windows_paste_with_a_line_break_never_runs_the_launch_menu` in `terminal_pty.rs` pins the paste
on the Windows pseudo-console, and the `terminal.rs` unit tests pin each burst rule on every
platform.

`tui_text_editing.rs` drives each key group in a real terminal for a launch-menu box, the Add path
box, the search box, an Entry settings box, and the multi-line Description box.
`text_editing_surfaces.rs` gives the same keys to the Rename dialog, the Preferences editor box, the
agent editor, the preset-name dialog, and the environment dialog.
