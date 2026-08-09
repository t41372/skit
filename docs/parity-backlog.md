# Version 0.4 parity backlog

Known gaps against the behavioral oracle (`origin/main@206f9ef`, Python `0.4.1.dev0`). Each item
names the oracle file and line. Remove an item only when a contract test pins the version 0.4
behavior.

## Localization catalog fidelity

`crates/skit-i18n/src/lib.rs` carries retranslated Chinese copy where version 0.4 ships a
translation. Measured once over 914 catalog rows: 334 had an English msgid that matches
`src/skit/locales/*/LC_MESSAGES/skit.po` exactly; of those, 154 zh-CN and 163 zh-TW translations
differed from the shipped text. A further 466 `.po` msgids had no exactly matching English row, so
some English copy is reworded as well. Treat those numbers as a snapshot, not a running total: each
slice that adds a section repairs the rows it touches, so the count moves without the gap closing.

The clearest symptom is term drift: version 0.4 renders "runner" as `执行器`, and 57 catalog rows
use `运行器`. `crates/skit-i18n/tests/catalog.rs` pins `Library` to `程序库`, where the `.po` says
`工具库`.

Product rule 1 makes the `.po` text authoritative for every sentence version 0.4 already ships.

## Add refuses an unreadable source with its own wording

`skit add <missing path>` reports `could not resolve <path>: <io error>`. Version 0.4 raises
`StoreError(gettext("File not found: %(path)s"))` (`src/skit/cli.py:418-422`,
`src/skit/store.py:461-462`). The exit code already matches at 1. The Rust text adds the operating
system reason, which is more information, but the sentence is not the one the catalog translates.

## Runner add refuses with the row-status wording

Version 0.4 keeps two closed sets: `_runner_reason` for a `runner add` refusal
(`src/skit/cli.py:3358-3378`) and `prompt_runner_row_reason` for a row status
(`src/skit/config.py:592-624`). `crates/skit-store/src/config.rs` now serves the row status from the
version 0.4 set, but `set_runner` still refuses with that same wording instead of the refusal set.

Example: an empty command refuses with `Type the agent's command, e.g. mycli run {{prompt}}` where
version 0.4 says `A runner needs a command — e.g. skit runner add mycli mycli run {{prompt}}`.

## The footer wears a border version 0.4 does not

Every screen draws its key rows inside a rounded box, which costs two rows of body on every
terminal. Version 0.4 docks a bare `KeysBar` with no border (`src/skit/tui_footer.py:97-108`, and
visible in every shipped frame under `docs/assets/`). Recovering those two rows would let the run
form show its argument tail without scrolling, exactly as the oracle's own demo frame does.

## The settings screen the user reaches is still the flat form

`skit-form::parameter_section` and `skit-ui::SettingsView` now model every shape version 0.4 draws:
a block-managed `ParamRow` with three editable axes (`src/skit/tui_settings.py:73-138`), a
hand-declared `DeclParamRow` (`:151-230`), the one-sentence explanation a reader-driven entry gets
instead of checkboxes (`:612-623`), and the read-only `· name (type)` lines of a reference entry
(`:597-606`).

`crates/skit-cli/src/cli.rs` still composes `tui_settings_form`, so none of it is on screen. The
composition root has to open `Screen::Settings` instead, which waits on the presets section.

Two earlier defects in this entry are fixed and have contract tests. `settings_parameter_fields`
emits only the controls a row offers, rather than fifteen `FormField::text` axes for every
parameter. The submit-time filter that dropped any declaration whose name the source also produced
is gone, and with it both of its races: a concurrent source edit changing which rows survive, and an
unreadable source silently widening the set that does.

## The parameter row prints its default differently

Version 0.4 renders the default inside a `ParamRow` label with Python `repr`
(`src/skit/tui_settings.py:100`), so a string reads `'world'` and a boolean reads `True`.
`crates/skit-form/src/parameter_section.rs` renders `world` and `true`.

The same label also shows a different value. Version 0.4 resolves it through
`analysis.effective_default`, which prefers the source's live literal and falls back to the value
the block cached (`:609-611`). The Rust row reads the block value only, so a script whose constant
moved after the block was written shows the stale number.

## The resync gives no report before the save

Version 0.4's `Ctrl+R` reads the definitions again immediately, keeps the result in memory, and
prints either the analyzer warnings or `Everything still matches the script.` above the save
(`src/skit/tui_settings.py:908-926`, shown at `:637`). A person sees what would change before
choosing to keep it.

Version 0.5 ticks a control that applies the resync when the save runs, so the report has nowhere to
appear. Building it needs a host round trip that returns the new declarations and warnings, and a way
to refresh the rows without discarding edits in the other sections. See `docs/behavior-changes.md`
for the control that exists today.

## A prompt cannot manage its detected variables from the screen

Version 0.4 lists a prompt's unmanaged `{{name}}` placeholders as tick-to-manage checkboxes under
the declared rows, caps the inline list at `LIST_PREVIEW_LIMIT`, says how many more there are, and
offers `Ctrl+O` to open the complete set in a modal (`src/skit/tui_settings.py:687-717`,
`:1140-1157`, `:1158-1180`). Each ticked candidate becomes a placeholder-delivery declaration that
is required, and secret when its name says so (`:1123-1139`).

Version 0.5 offers only the add-a-parameter box, and `ParamDecl::new` builds a flag-delivery
declaration, so a name typed there does not become a prompt placeholder.
