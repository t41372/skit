# Version 0.4 parity backlog

Known gaps against the behavioral oracle (`origin/main@206f9ef`, Python `0.4.1.dev0`). Each item
names the oracle file and line. Remove an item only when a contract test pins the version 0.4
behavior.

## Localization catalog fidelity

`crates/skit-i18n/src/lib.rs` now uses the shipped Chinese text of
`src/skit/locales/*/LC_MESSAGES/skit.po` for each row whose English matches a msgid. A row keeps a
different text only when the catalog uses one term for one meaning (for example `执行器`/`執行器`
for runner, `工具库`/`工具庫` for Library, `参数组合`/`參數組合` for preset, and `項目` for entry in
zh-TW), when the 0.4 sentence puts positional `{}` values in a different order, when the English
changed, or in a few rows where the newer text is clearer (for example `编辑源文件` for
`Edit source`). Rows with reworded English reuse the 0.4 wording where the meaning is the same.

These items remain:

- `crates/skit-i18n/tests/catalog.rs` pins the zh-TW row `✓ skit read this script's own arguments
  ({} field)…` to `✓ skit 已讀取這支腳本自己的參數（1 個欄位）。執行時會開啟表單，無需記憶指令。`.
  Version 0.4 says `✓ skit 看懂了這支腳本自己的參數（共 1 項）。執行時會開表單——不用背任何指令。`.
  The plural row follows the pinned row.
- No contract test pins the restored rows as a set. A later edit can bring the drift back.
- The Chinese README and documentation pages use other terms than the catalog: `运行器` for
  runner, `程序库`/`程式庫` for Library, `代理` for agent, `敏感值` for secret (zh-CN),
  `预设`/`預設` for preset, and `删除` for removing an entry (zh-CN). The zh-TW pages also use
  `引數`, `預留位置`, `相依`, and `固定` where the catalog uses `參數`, `佔位符`, `依賴`, and
  `釘選`, and `README.zh-TW.md` still says `條目` in three places.

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

These are the last two rows between the entry-settings frame and the oracle's. With the shared
header gone and the description box back to three rows, that screen now reaches its parameter
section and one row past it; the oracle reaches the same row with two rows to spare.

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
