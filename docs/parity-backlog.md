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

- No contract test pins the restored rows as a set. A later edit can bring the drift back.
- The Chinese README and documentation pages use other terms than the catalog: `运行器` for
  runner, `程序库`/`程式庫` for Library, `代理` for agent, `敏感值` for secret (zh-CN),
  `预设`/`預設` for preset, and `删除` for removing an entry (zh-CN). The zh-TW pages also use
  `引數`, `預留位置`, `相依`, and `固定` where the catalog uses `參數`, `佔位符`, `依賴`, and
  `釘選`, and `README.zh-TW.md` still says `條目` in three places.

Product rule 1 makes the `.po` text authoritative for every sentence version 0.4 already ships.
