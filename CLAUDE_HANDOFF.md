# Handoff: complete the Rust rewrite

This is a temporary work file. Delete it before the pull request. Do not include it in the merge.

## 1. Objective and work instructions

### 1.1 Objective

Finish the complete Rust and Ratatui replacement of skit. The merged pull request removes the
Python implementation. Maturin stays only as the binary-wheel compatibility layer for PyPI and
`uv tool install skit-cli`. Version 0.4.0 upgrades directly to version 0.5.0. The
frontend-neutral `skit-ui` seam stays ready for a future Tauri frontend. Do not leave an
intermediate migration state.

### 1.2 Product rules

Read `AGENTS.md` first and obey it exactly. It is the authority. This file does not replace it.

Its **Trust model** is critical. Local scripts, arguments, templates, prompts, environment
values, output, and logs are trusted. Do not add sanitization, escaping, character blocks,
permission policies, secret-handling policies, or other threat mitigations for them. Do not
refuse an operation because a trusted script could run a command.

Security work is in scope only when it does one of these things:

- It keeps the exact version 0.4 behavior.
- It prevents skit from changing files outside its own directories.
- It prevents skit from losing, corrupting, or partially committing user data.
- An explicit `AGENTS.md` product rule requires it.

Format validation is a correctness concern when a runtime or a file format needs valid input. Do
not expand format validation into a threat model.

### 1.3 How to work

- **Use TDD.** Write a failing contract test before each implementation change. Then write the
  smallest change, and refactor while the tests stay green. Add refusal, corruption, race, and
  rollback tests for each stateful boundary.
- **Write ASD-STE100 English.** This applies to new comments, user copy, errors, and
  documentation. Use short direct sentences, one term for one meaning, and active voice. Text
  that version 0.4 already shipped keeps its wording; parity outranks style there.
- **Use current 2026 dependencies and syntax.** Do not add an older API when a current one
  exists.
- **Keep every gate hard.** Do not weaken `scripts/check_coverage.sh`, the mutation gate, or any
  other checker to make it pass. Add behavior tests instead of mutation-specific hacks.
- **Prove each claim.** Run the command and read its output before you report a result. Do not
  call a gate green from memory. Section 4 is a record of measured results, not of intent.
- **Order an independent read-only full-tree review** after the implementation work. Fix every
  correctness, parity, compatibility, packaging, frontend, and documentation finding. Do not add
  hardening that the Trust model excludes.
- **Commit, push, and open a non-draft pull request only after every hard gate passes** and the
  final review is clean. Do not claim the rewrite is complete before that.

## 2. Repository state

- Repository: `/home/ubuntu/coding/skit`
- Branch: `rewrite/rust-ratatui-complete-20260808-codex`
- The branch has commits. `git log` shows the work. This file is the newest commit.
- The worktree is clean, or holds only your own new work.
- `mutants.out/` and `mutants.out.old/` are mutation artifacts. `.gitignore` excludes them. Do
  not commit them and do not use them as evidence.

## 3. What the previous sessions completed

### 3.1 The Rust workspace

Eleven crates replace the Python program: `skit-domain`, `skit-application`, `skit-language`,
`skit-form`, `skit-store`, `skit-runtime`, `skit-ui`, `skit-tui`, `skit-cli`, `skit-i18n`, and
`skit-benchmarks`. `docs/design/rust-rewrite.md` states the architecture and the compatibility
boundary. The Python implementation, its tests, and its tooling are deleted.

### 3.2 Typed message localization

Version 0.4 translated rendered text by substring replacement. That method rewrote any catalog
fragment anywhere in a string. It corrupted user data and ordinary English words. One example
under `SKIT_LANG=zh-CN`:

```
invalid PEP 440 versi开启 c开启straint "否t a versi开启"
```

`on` inside `version` and `not` inside the user's own value both changed.

The replacement is in `crates/skit-i18n/src/lib.rs`:

- `Message` holds a stable English template, its ordered values, and optional nested messages.
  `Message::localize` translates the template with an exact catalog lookup, then inserts the
  values. A value never reaches the translator.
- `Localize` is a trait. Each user-visible error implements it. The `message` method matches on
  every variant, so a new variant needs a new template. The compiler enforces this.
- `render` is now only for text that a framework composes. It replaces only rows marked
  `composable`, and only at word boundaries.
- `skit-cli` translates the Clap command tree before it parses, with exact lookups. A token such
  as `--help` cannot change.

Tests that hold this contract:

- `crates/skit-i18n/tests/catalog.rs` walks each crate `src` tree, collects every
  `Message::new` template outside `#[cfg(test)]` modules, and fails when the catalog has no
  complete row.
- `crates/skit-{domain,application,language,runtime,store}/tests/localization.rs` build every
  error variant. Each test asserts that the English text equals the `thiserror` display, that
  each locale fills every hole, and that each value stays byte-identical.
- `crates/skit-cli/tests/typed_error_locales.rs` drives the real binary in all three locales.

The catalog has about 456 rows. Every row has complete Simplified and Traditional Chinese text.

### 3.3 Documentation locales

`docs/lib/i18n.ts` publishes `en`, `zh-CN`, and `zh-TW`. Each of the nine pages has a
`.zh-CN.mdx` and a `.zh-TW.mdx` sibling, plus `meta.zh-CN.json` and `meta.zh-TW.json`. Each
translated heading carries an explicit `[#anchor]` that equals the English slug, so a deep link
survives a language change. `docs/scripts/sync-readme.mjs` copies all three READMEs.
`docs/lib/layout.shared.tsx` translates all 50 Fumadocs chrome keys for both Chinese locales.

### 3.4 Coverage

`scripts/check_coverage.sh` reported 189 uncovered executable lines at the start. It now
reports `complete executable-source line coverage`. The work added contract tests and removed
unreachable code. Do not weaken the checker.

### 3.5 Defects that four independent reviewers proved, and that are now fixed

Each row was reproduced before the fix and has a regression test.

| Area | Defect | Effect |
| --- | --- | --- |
| `skit-runtime/src/javascript_deps.rs` | `require_entry_directory` ran after `recover_dependency_backup` | A symlinked entry directory let skit write outside its own directories |
| `skit-runtime/src/javascript_deps.rs` | `clear_javascript_dependencies` had no directory guard | The same symlink let skit delete user files outside its directories |
| `skit-store/src/config.rs` | `write_runners` replaced the stored `[[prompt.runners]]` list | Hand-written runners disappeared and the seeds took their place |
| `skit-store/src/config.rs` | `remove_runner_row` seeded the defaults before it used the index | `--row 2` deleted a different row than `runner list --all` showed |
| `skit-store/src/read.rs` | A metadata mtime failure returned before `read_entry` | A valid entry disappeared from `list`, with exit code 0 |
| `skit-store/src/mutations.rs` | `rebuild_registry` propagated one `project` failure | One unusable entry stopped the complete rebuild |
| `skit-store/src/paths.rs` | `is_support_file` missed `.run-*` and `.*.tmp` | An interrupted write blocked every launch of that entry, permanently |
| `skit-language/src/lib.rs` | The Boolean literal followed the stored literal spelling | `FLAG = 1` with `--set FLAG=false` wrote `false` into Python, so Python raised `NameError` |
| `skit-language/src/lib.rs` | `strip_skit_section` kept `[tool.skit.<name>]` | Every source operation failed on version 0.4 data that holds such a table |
| `skit-i18n/src/lib.rs` | Four Chinese rows expected the path before the operation | `无法在 lock 处/path配置` instead of the verb and the path in their places |
| `skit-cli/src/cli.rs` | `render` instead of `text` for five scalar rows | `yes`, `no`, `on`, `off`, and `not set` stayed English in `show` and `params` |
| `skit-i18n/src/lib.rs` | `detect_locale` ignored `zh-HK`, `zh-MO`, and `zh-SG` | Hong Kong, Macau, and Singapore fell back to English |

The Chinese row order defect is important for you. The per-crate `localization.rs` helpers used
`text.contains(value)`, which cannot see a wrong value order. They now assert one complete
rendering for a multi-value message. Keep that pattern.

## 4. Gate results

All results are from Linux x86-64 in this worktree.

| Gate | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --all --check` | pass |
| Whitespace | `git diff --check` | pass |
| Build | `cargo build --locked --workspace --all-features` | pass |
| Tests | `cargo test --locked --workspace --all-targets --all-features` | pass, 556 tests |
| Clippy | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| Rustdoc | `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps` | pass |
| Coverage | `cargo llvm-cov …` then `bash scripts/check_coverage.sh lcov.info` | pass before section 5.1 edits; **re-run it** |
| Checker self-tests | `scripts/test_coverage.sh`, `scripts/test_english.sh`, `scripts/test_tooling_contracts.sh`, `benchmarks/test.sh` | pass |
| English | `bash scripts/check_english.sh` | pass |
| Supply chain | `cargo deny --locked check`, `cargo audit --deny warnings`, `zizmor .github/workflows` | pass |
| Documentation | `cd docs && npm ci && npm run types:check && npm run build` | pass, 34 pages, every link and anchor resolves |
| Benchmarks | `bash benchmarks/run.sh pr .bench target/release/skit` then `bash benchmarks/check.sh …` | pass, every budget |
| Benchmarks | `cargo bench --locked --workspace --all-features` | pass |
| Packaging | `maturin build`, `maturin sdist`, `uv tool install` for both | pass, both run and both install the Agent Skill |
| Packaging | `cargo test -p skit-language --test corpus` inside the extracted sdist | pass |
| **Mutation** | `cargo mutants …` | **NOT COMPLETE. See section 5.1.** |

`hyperfine` is not on this machine by default. Install the pinned build that
`.github/actions/install-hyperfine/action.yml` names.

## 5. What is not done

Work through the sections in order. Section 5.1 is the release blocker.

### 5.1 Mutation testing has no clean run

This is the one hard gate that is not green. `AGENTS.md` demands zero survivors.

**Status.** A full scan examines 3230 mutants. The last scan reached about 390 outcomes and then
stopped. It reported 20 survivors, all in `crates/skit-cli/src/cli.rs`. The scan never reached
the other ten crates, so the true survivor count is unknown and is probably much larger.

**Throughput.** About 22 to 25 outcomes each minute with `--jobs 6` on 8 cores. A complete scan
needs about two hours. Many mutants are `Unviable`, which is fast.

**Two operational rules.**

1. Do not edit a source file while a scan runs. `cargo-mutants` copies the tree, so the line
   numbers in `mutants.out/missed.txt` belong to the copy, not to your edits.
2. Do not run another heavy job at the same time. It slows the scan and it is hard to separate
   contention from a real timeout.

**About timeouts.** An earlier scan reported 17 timeouts. They are not contention. Each one
mutates a loop counter in `split_windows_arguments` and makes an infinite loop. `cargo-mutants`
counts a timeout as caught, so a timeout does not break the gate. It costs three times the
timeout in wall-clock time.

**Survivors from the partial scan.** Line numbers are from before the section 3.5 fixes, so
find each one by function name.

| Location | Mutation | State |
| --- | --- | --- |
| `add_command`, two `\|\|` operators | replace `\|\|` with `&&` | open |
| `validate_prompt_runner` | replace body with `Ok(())` | **closed** by `surface_edges.rs::adding_a_prompt_refuses_a_runner_that_is_not_configured` |
| `run_entry`, three `\|\|` operators | replace `\|\|` with `&&` | open |
| `interactive_run_form`, delete `!` | delete `!` | open |
| `interactive_run_form`, `!=` to `==` | replace `!=` with `==` | open |
| `interactive_run_form`, `==` to `!=` | replace `==` with `!=` | open |
| `split_editable_arguments`, three variants | replace body | open |
| `join_editable_arguments`, four variants | replace body | open |
| `split_windows_arguments`, `<` to `<=`, three sites | replace `<` with `<=` | **closed**; the outer loop is now `loop`, and `windows_argument_encoding_round_trips_every_quoting_shape` covers the rest |
| `join_windows_arguments`, `*` to `/` | replace `*` with `/` | **closed** by the same round-trip test |

**How to kill the open ones.** Most need a pseudo-terminal test.
`crates/skit-cli/tests/terminal_pty.rs` already has the helpers: `run_in_pty`,
`run_plain_in_pty`, and `run_pty`. The plan below is analysed but not written.

- The two `add_command` operators and the three `run_entry` operators guard
  `no_input || !stdin.is_terminal() || !stdout.is_terminal()`. To kill the first operator, run
  inside a pseudo-terminal with `--no-input` and assert the non-interactive result. To kill the
  last operator, you need exactly one terminal. Run `sh -c 'skit … < /dev/null'` inside a
  pseudo-terminal. Then stdout is a terminal and stdin is not. Add a helper that starts `sh`
  instead of the binary. This also tests the documented rule that interactive mode needs both
  standard streams to be terminals.
- The three `interactive_run_form` mutations control the prompt-runner list. The pinned runner
  must come first, the other configured runners must stay selectable, and an entry with no pin
  must default to the first configured runner. Use `run_plain_in_pty`, because the plain form
  prints the choices. `write_pinned_prompt_entry` already exists in that file.
- `split_editable_arguments` and `join_editable_arguments` carry the remembered `--` tail
  through the interactive form. Run an entry once with `-- 'two words' single`, then run it
  again inside a pseudo-terminal and accept every prefilled value. Assert that the dry-run
  output holds both original arguments. That kills all seven mutants.

**Recommended order.** Iterate one file at a time with
`cargo mutants … --file <path>` and `-F <regex>`. A whole-workspace scan for each fix is too
slow. Run one complete scan at the end for the record.

### 5.2 Review findings that are still open

Four independent read-only reviewers examined the tree. Section 3.5 lists what is fixed. Every
item below is still open. Each one names the file and the reproduction.

#### 5.2.1 Localization, correctness

1. **Clap error text still rewrites user arguments.** `crates/skit-cli/src/cli.rs:82` sends the
   complete Clap error through `render`. Clap has already put the user's argument inside it.
   ```
   $ SKIT_LANG=zh-CN skit "Print help"
   error: unrecognized subcommand '显示帮助'
   $ SKIT_LANG=zh-TW skit "Entry added"
   error: unrecognized subcommand '項目已新增'
   ```
   The `render` doc comment claims that typed messages keep user values out of the translation,
   but this call site feeds it user data. Consider translating only the framework headings that
   Clap emits on their own lines.
2. **`to_string()` forces English on types that implement `Localize`.**
   - `crates/skit-tui/src/terminal.rs:73` — `Action::SetStatus(error.to_string())`. The bound is
     `E: Display`, so every in-TUI failure shows English. `Display for Message` hardcodes
     `Locale::En`.
   - `crates/skit-store/src/config.rs:712` — `validate_runner(runner).err().map(|e| e.to_string())`
     discards the `Message`. `SKIT_LANG=zh-CN skit runner list --all` prints
     `a prompt runner command needs {{prompt}} exactly once after the program`, although that
     row is translated.
   - `crates/skit-store/src/read.rs:202,222,239,303` — `Diagnostic.message = error.to_string()`.
     English reaches `skit list` (`warning: {}`), `skit doctor` (`WARN {}`), and the TUI health
     report.
3. **`doctor_launch_block` builds English strings by hand.**
   `crates/skit-cli/src/cli.rs:3334,3343,3350,3356`. `SKIT_LANG=zh-CN skit doctor` prints
   `警告 rb：运行将无法启动：required program was not found: ruby`. The catalog already holds
   `required program was not found: {}`, `unknown entry kind: {}`, and a near duplicate of
   `prompt runner {} is not configured`.
4. **Text with no catalog row.**
   - `crates/skit-store/src/config.rs:711` — `runner row needs a name and a string argv array`.
   - `crates/skit-store/src/config.rs:718` — `format!("row {index}")`, which reaches doctor.
   - `crates/skit-cli/src/cli.rs:2986` — `row.reason.as_deref().unwrap_or("valid")`.
   - `crates/skit-store/src/mutations.rs:661` — `format!("{primary}; rollback also failed: {rollback}")`.
5. **English fragments arrive as `Message` values, so one sentence holds two languages.**
   These are skit's own words, not user data, so product rule 1 applies to them.
   | Site | Output under `SKIT_LANG=zh-CN` |
   | --- | --- |
   | `cli.rs:2860` `assignment`, called at `:2525` and `:2535` | `environment target 需要 NAME=VALUE` |
   | `cli.rs:2840` `RunnerSelection::label` | `未知的提示词运行器：row 99` |
   | `cli.rs:3033` and `:3099` `ConfirmationRequiredFor` | `runner removal 需要确认；请传入 --yes` |
   | `cli.rs:4495` `source_error`, 13 call sites | `无法read /path：…` |
   The same class covers the `operation` verb inside every `could not {} … at {}: {}` row.
   `SKIT_DATA_DIR=/dev/null/nope SKIT_LANG=zh-CN skit list` prints `无法scan …`. The verb set is
   closed and skit owns it, so give each verb a catalog row and nest it with `Message::nested`.
6. **Clap's own diagnostics never translate.** Only eight framework fragments are `composable`.
   `error:`, `tip:`, `For more information, try '--help'.`, and every message body stay English.
   `crates/skit-cli/src/cli.rs:70` calls `error.print()` on the completion path, which skips
   localization completely. This is the largest untranslated surface that remains.

#### 5.2.2 Localization, parity and polish

7. `crates/skit-i18n/src/lib.rs:379` — `Secret: yes` uses `机密` while every other secret row
   uses `敏感值`. One term for one meaning.
8. zh-CN term collisions: `schema` and `mode` both render as `模式`; `Kind` and `Type` both
   render as `类型`; `Choices` and `Options:` both render as `选项`. zh-TW uses `結構` for
   schema, so the two locales also disagree.
9. zh-TW uses `預設值` for default and `預設` for preset in one sentence. zh-CN avoids this with
   `默认值` and `预设`. `README.zh-CN.md` uses `组合` for preset six times against `预设` twice.
   `README.zh-TW.md` uses `組合` seven times against `預設` five times.
10. `crates/skit-i18n/src/lib.rs:203` — the `Usage:` row keeps the ASCII colon, and the
    full-width `：` already carries a space. Output shows `用法： skit` with a double gap.
11. `crates/skit-tui/src/lib.rs:225` — `.to_lowercase()` applies to the complete composed line,
    so English shows `storage mode: copy`. Lowercase only the debug value. The same block uses
    an ASCII `": "` separator with Chinese labels.
12. `crates/skit-i18n/src/lib.rs:1400` and `crates/skit-runtime/src/launch.rs:135` — the message
    says the marker is `{prompt}`, but the real runner marker is `{{prompt}}`.
13. `crates/skit-i18n/src/lib.rs` `replace_words` — `word_character` is ASCII only, so
    `render(ZhCn, "Print helpé")` gives `显示帮助é`. The `.map_or(1, …)` default is unreachable
    and will show as a live mutant.
14. `crates/skit-i18n/tests/catalog.rs:88` — the i18n test reads `skit-cli/src/cli.rs` with
    `include_str!`, which inverts the dependency direction. `message_templates` indexes
    `bytes[end]` with no bound check and panics on an unterminated or raw string literal.
    `without_test_modules` counts braces inside string and character literals.

#### 5.2.3 CLI parity

15. **`skit edit` does not refuse `exe` and `command` entries.**
    `crates/skit-cli/src/cli.rs:1979`. `payload_path` returns `Ok` for every reference entry, so
    the guard never runs. `skit add --cmd` stores `mode = "reference"` and an empty source.
    ```
    skit config editor "/bin/echo EDITOR-OPENED"
    skit edit cmd   --no-input → "EDITOR-OPENED "                     exit=0
    skit edit mybin --no-input → "EDITOR-OPENED /tmp/…/mybin"         exit=0
    ```
    `docs/content/docs/cli.mdx:105` says skit refuses both. `show_source_text:1467` has the
    guard, which shows the intent.
16. **`skit show` is the only command that fails on a missing or non-UTF-8 prompt payload.**
    `crates/skit-cli/src/cli.rs:1470`. Prompt entries get a strict read. Every other kind gets
    `unwrap_or_default()` and `from_utf8_lossy`. `skit show promptref --json` exits 125 while
    `skit params` and `skit doctor` both work. The diagnostic command is the one that stops.
17. **The prompt copy path hardcodes `prompt.md`.** `crates/skit-cli/src/cli.rs:1471` uses
    `stored_filename("prompt")` and skips the single-file fallback in `payload_path`. A stored
    prompt named `prompt.txt` makes `show --json` exit 125 while `params --json` still works.
18. **`skit doctor` exits 1 on an empty library.** `crates/skit-cli/src/cli.rs:3189` sets
    `uv_required = entries.is_empty() || any python`. `docs/content/docs/cli.mdx:38` says
    `1 = required but missing`. An empty library needs no runtime. Decide which one is right.
    `edge_workflows.rs:369` freezes the current behavior, so fix the test with the code.
19. `crates/skit-cli/src/cli.rs:1153` — `split_windows_arguments` treats `"` as a pure toggle.
    Windows turns `""` inside a quoted run into one literal `"`. `join_windows_arguments` never
    emits that form, so the round trip holds. Only a hand-typed value differs.

#### 5.2.4 Store and runtime, remaining

20. **`meta.toml` and `config.toml` lose comments on every write.**
    `crates/skit-store/src/config.rs:411` and `crates/skit-store/src/mutations.rs:568` use the
    `toml` value tree, so a write reserializes the complete document. `# my comment` disappears
    and keys sort alphabetically. Unknown fields do survive. Decide whether version 0.4 treats a
    comment as user bytes. `toml_edit` keeps them.
21. `crates/skit-store/src/mutations.rs:550` — `encoded.parse::<toml::Table>().expect(…)`. The
    reviewer could not reach it, and analysed both the value-after-table order and the parser
    recursion limit. Even so, a write path for authoritative version 0.4 data should return the
    typed error that the two neighbouring lines already build.
22. `crates/skit-runtime/src/javascript_deps.rs:458,469,476` — the rollback is
    `remove_dependency_items(…).and_then(|()| recover_dependency_backup(…))`. If the removal
    fails, the recovery never runs. The error does surface and the next call self-heals. Run the
    recovery unconditionally.
23. `crates/skit-store/src/mutations.rs:283` — a kill before `StagedDirectory::drop` leaves
    `.staging/<slug>-<id>`, and nothing sweeps `.staging`.
24. `crates/skit-cli/src/run/command.rs:611` — `payload_path` runs before
    `sweep_staged_sources`. Section 3.5 removed the failure this caused, so this is now only an
    ordering wart. Move the sweep first.
25. **`skit rename` never derives a new slug.** `crates/skit-store/src/mutations.rs:125` sets
    only `meta.name`. The CLI help and the catalog both say "Rename one entry and derive its new
    slug". `skit rename thing "Thing2"` prints `Renamed: Thing2 (thing)`. Decide whether the
    text or the behavior is wrong. Version 0.4 is authoritative.
26. **New entries get no add timestamp.** `crates/skit-store/src/mutations.rs:275` writes
    `added_at: String::new()`. The version 0.4 fixtures in `skit-store/tests/file_store.rs:27`
    carry real RFC-3339 timestamps.

#### 5.2.5 Documentation site and packaging

27. **Chinese search returns nothing.** `docs/app/api/search/route.ts:8` sets
    `language: 'english'` for the complete index. The Orama English tokenizer splits on
    whitespace, and Chinese has none. The built index holds 0 CJK tokens against 3186 CJK runs
    in the document store. The Fumadocs types say the default multilingual tokenizer needs no
    configuration and that `localeMap` is deprecated, so removing the `language` option is
    probably the complete fix. Rebuild and count CJK tokens in `docs/out/api/search` to confirm.
28. **Every Chinese page serves English markdown.** `docs/lib/source.ts:23`
    `getPageMarkdownUrl` builds the URL from `page.slugs` only, and
    `docs/app/llms.mdx/docs/[[...slug]]/route.ts:9` calls `source.getPage(slug)` with no locale.
    `docs/out/zh-CN/docs/cli/index.html` points at `/skit/llms.mdx/docs/cli/content.md`, whose
    first line is `# CLI reference (/en/docs/cli)`. This breaks Copy Markdown, View as Markdown,
    and every Open in… action. `docs/app/og/docs/[...slug]/route.tsx` has the same defect for
    the social image: its `generateStaticParams` returns `lang: page.locale`, but the route has
    no `[lang]` segment. Add a locale segment to both routes and include the locale in
    `docs/lib/shared.ts` routes.
29. **The site does not rebuild on a translated README.** `.github/workflows/docs.yml:12` and
    `:20` list `README.md` but not `README.zh-CN.md` or `README.zh-TW.md`. Those two files are
    the complete content of `/zh-CN/docs` and `/zh-TW/docs`.
30. **The Chinese READMEs hold stale version 0.4 copy.** `README.zh-CN.md:77` and
    `README.zh-TW.md:77` describe a review page for placeholder selection. The Rust add flow has
    no review page. `crates/skit-cli/src/cli.rs:1827` manages every detected placeholder when
    there are 30 or fewer. `README.md:82` was rewritten; the translations were not. They also
    drop the `skit params` pointer, so the Chinese reader learns no way to manage fields.
31. `README.zh-CN.md:71,79` and `README.zh-TW.md:71,79` link only to `/en/docs/…` and label the
    links "文档（英文）". The Chinese documentation now exists.
32. `README.zh-CN.md:4` and `README.zh-TW.md:4` miss the CodSpeed badge that `README.md:5` has.
33. `docs/README.md:15` says "English-only for now — … no translated content exists yet." That
    is false. It also never names `meta.zh-CN.json`, `meta.zh-TW.json`, or
    `scripts/sync-readme.mjs`.
34. `CONTRIBUTING.md:117` describes a single-locale site. Nothing tells a contributor that a page
    change needs the two translated siblings and the two translated READMEs, which product
    rule 1 makes mandatory.
35. `docs/content/docs/cli.zh-CN.mdx:82,92` and the zh-TW file use `[#skit-list-show]` and
    `[#skit-remove-rename-describe-edit]`. The English slugs are `skit-list--show` and
    `skit-remove--rename--describe--edit`, because a slash collapses to two hyphens. The other
    33 explicit ids match. Nothing links to these two today.
36. **The `skit params` table misses seven flags** in `docs/content/docs/cli.mdx:112` and in both
    translations: `--binding`, `--multiple`, `--no-multiple`, `--repeat`, `--no-repeat`,
    `--env-target`, and `--action`. Four of them set keys that the same page documents in the
    `show --json` schema. `--data-dir`, `--install-completion`, and `--show-completion` appear
    in no page, no README, and no `SKILL.md`. The page description says "Every command, flag, and
    exit code."
37. **Localized placeholder tokens are not valid placeholders.** `prompts.zh-CN.mdx:23,29,70`,
    `prompts.zh-TW.mdx`, `script-types.zh-*.mdx:22`, and both Chinese READMEs write
    `{{占位符}}`, `{{洞}}`, and `{{預留位置}}`. `valid_identifier` in
    `crates/skit-language/src/lib.rs:1647` needs an ASCII first character, so skit never detects
    those. Keep the English token inside the braces.
38. `scripts/record_demo.sh:49` records only `en` and `zh-TW`. `README.zh-CN.md:32` and
    `README.zh-TW.md:32` embed the same video, so the Simplified Chinese README shows a
    Traditional Chinese interface.
39. **`SKILL.md` has no synchronization test, and `CONTRIBUTING.md:113` says it does.** The only
    content assertion is one hard-coded string in
    `crates/skit-cli/tests/product_contract.rs:245`. `AGENTS.md` says to keep the commands
    synchronized with the real CLI, so add the gate and fix the claim.
40. `skills/skit/SKILL.md:27` embeds `繁體中文, 简体中文` in a file that product rule 1 makes an
    English-only machine contract.
41. `skills/skit/SKILL.md:114` says `126 | target exists but is not executable`. Its own prompt
    section and `docs/content/docs/cli.mdx:35` give 126 a wider meaning. An agent that reads only
    the table will misdiagnose a missing runner.
42. `.cargo/mutants.toml` sets `timeout_multiplier = 3.0`. `.github/workflows/mutation.yml:35`
    and the `AGENTS.md` command do not pass it, so CI can report a timeout that a clean local run
    never shows.
43. `scripts/test_tooling_contracts.sh:13` pins action SHAs for four workflows but not for
    `docs.yml`, `benchmark.yml`, `benchmark-nightly.yml`, or `benchmark-compare.yml`. Also
    `zizmor .github/workflows` does not scan `.github/actions/install-hyperfine/action.yml`.
44. `scripts/check_english.sh:19` scans `docs/app`, `docs/components`, `docs/content`,
    `docs/design`, `docs/lib`, `docs/public`, and five named files. It never scans `crates/**`
    or `docs/scripts/**`. Six English error strings use the contraction "isn't"
    (`skit-application/src/value_resolution.rs:20,36`, `tokens.rs:31,44`, and the matching
    catalog rows). Decide first whether those are exact version 0.4 text. Version 0.4 parity
    outranks style for text that already shipped.
45. `.gitignore:18` explains the `.claude/` entry with Python tooling names. `docs/design/multilang.md`,
    `docs/design/path.md`, and `docs/design/prompt.md` still describe `src/skit`,
    `tests/test_argspec*.py`, Babel extraction, and pytest gates. `multilang.md` is the i18n
    design document and it now contradicts `crates/skit-i18n`.

### 5.3 Tests that pass for the wrong reason

Fix these with the behavior they claim to hold.

46. `crates/skit-cli/tests/product_contract.rs:432` — `write_command_entry` hand-writes
    `mode = "copy"` for a command entry, but `skit add --cmd` produces `mode = "reference"`. The
    test never reaches the real path, which is why finding 15 stayed hidden.
47. `crates/skit-cli/tests/edge_workflows.rs:611` — the `edit no-source` case passes because
    `config editor false` makes `/bin/false` exit 1. Deleting the refusal keeps the test green.
48. `crates/skit-cli/tests/edge_workflows.rs:512` — `deps demo --clear --dep x` passes because
    the kind guard fires first. Deleting the conflict check keeps the test green.
49. `--clear` and `--clear-needs` have no post-condition assertion anywhere
    (`edge_workflows.rs:516,532,533,540`). Making either one a no-op keeps the suite green. The
    same holds for the eight-axis `params` call at `edge_workflows.rs:466`, which checks only the
    exit code.
50. `crates/skit-cli/tests/typed_error_locales.rs:1` claims that typed errors reach the user in
    every locale. It covers five error families out of about 140 templates, and none of them
    holds an injected English value, so finding 5 is invisible to it.
51. `crates/skit-cli/tests/mutations_cli.rs:6` is the only CLI test file that sets no
    `SKIT_CONFIG_DIR` and no `HOME`. It resolves the real `~/.config/skit`. The commands it runs
    are read-only today, so this is a latent hazard, not a live violation of product rule 6.
52. `crates/skit-cli/src/cli.rs:2308` and `:2627` — `write_managed_params` runs twice for each
    `params` source operation. It is idempotent, but the second parse is what turned finding 2 in
    section 3.5 from data corruption into a refusal. Correctness should not depend on a redundant
    call.
53. `crates/skit-language/src/lib.rs:632` — `json_to_toml` maps an array to a TOML array, and
    `json_toml_literal` maps the same array to a string that holds JSON. Both only ever receive
    `to_block_map()` output, which emits scalars, so the divergence is unreachable. Those arms
    are dead code that survives mutation testing.

### 5.4 Sequence that remains

1. Read `AGENTS.md`. Inspect the complete diff with `git log -p 070cbd5..HEAD`. Reproduce the
   section 4 gates yourself, so you know the true starting point.
2. Re-run coverage after each change. Section 3.5 changed many files.
3. Close the mutation gate. Section 5.1 has the plan and the measured throughput.
4. Fix section 5.2 and section 5.3.
5. Re-run every gate in `AGENTS.md`.
6. Push the branch. Collect Linux, macOS, and Windows CI evidence.
7. Order one more independent read-only review. Fix what it finds.
8. Inspect the complete diff. Delete this file and every mutation artifact.
9. Open a non-draft pull request.
10. Report exact gate evidence and every behavior change. Do not call the rewrite complete
    before every hard gate is green and the final review passes.

## 6. Working notes

- `cargo build --locked` fails when the lock file needs an update. Use `--offline` for local
  iteration and `--locked` for a gate.
- The four reviews ran as read-only agents with one scoped area each: localization, store and
  runtime data safety, CLI and language parity, documentation and packaging. That split worked.
  Each reviewer proved its findings with a sandbox under `/tmp` and the release binary.
- Use `mktemp -d` for `SKIT_DATA_DIR`, `SKIT_CONFIG_DIR`, and `SKIT_STATE_DIR` in every manual
  check. Product rule 6 forbids changes outside skit's own directories.
