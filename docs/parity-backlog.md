# Version 0.4 parity backlog

Known gaps against the behavioral oracle (`origin/main@206f9ef`, Python `0.4.1.dev0`). Each item
names the oracle file and line. Remove an item only when a contract test pins the version 0.4
behavior.

## Localization catalog fidelity

`crates/skit-i18n/src/lib.rs` carries retranslated Chinese copy where version 0.4 ships a
translation. Of 914 catalog rows, 334 have an English msgid that matches
`src/skit/locales/*/LC_MESSAGES/skit.po` exactly; of those, 154 zh-CN and 163 zh-TW translations
differ from the shipped text. A further 466 `.po` msgids have no exactly matching English row, so
some English copy is reworded as well.

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

## Settings offers editable axes it silently discards

`crates/skit-cli/src/cli.rs` renders fifteen `FormField::text` axes for every parameter, whatever
its provenance, and `tui_submit_settings` then drops any declaration whose name the source also
produces. A user edits `type`, `default`, `choices` or `help` on a parser-owned row, gets a success
result and no diagnostic, and nothing is written.

Version 0.4 never offers those axes. `self._specs` is the block-managed set alone
(`src/skit/tui_settings.py:610-611`), and each becomes a `ParamRow` whose name, type and default
are read-only text beside a keep checkbox — only the form label, the secret flag and the
environment source are editable (`:73-138`). A hand-declared row is a `DeclParamRow`, where type,
default, choices, help, flag and required are fields and delivery is read-only header text
(`:151-230`).

A parameter the script's own reader declares is not a row at all. A CLI-driven entry gets one
sentence instead, because managing a hardcoded constant would write a block that shadows the
script's own form (`:612-623`). A reference-mode entry renders read-only `· name (type)` lines and
nothing editable (`:597-606`).

Two aggravating details need their own assertions once the rows carry provenance: the discard set
is rebuilt from a fresh source read at submit time, so a concurrent edit changes which rows vanish;
and an unreadable source yields an empty set, so the same edit persists or disappears depending on
whether the file happened to be readable. The fix is the row model, not a wider filter.
