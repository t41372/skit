# UI review

## Findings

### P2 — The agent-skill picker title is not localized

The Simplified Chinese and Traditional Chinese picker title says `教 AI agent 使用 skit`. The word `agent` is English. The adjacent visible command correctly uses `代理`, so this is not a product term that must remain English.

Evidence: `zh-cn-120x30` checkpoint 52 presents styled frame `e3200220313b062f4a90cc9a066433fdde7b97959d5ef07881ace71cf49c76be`; `zh-tw-40x40` checkpoint 52 presents `b47a2a77ca718aacee250fd263cba6ddf12838668493bdb745cbc4ffad62d8d2`. Both show the picker after the same mouse action. Checkpoint 50 consumes the `install_agent_skill` target. Checkpoint 51 reduces it to `discover_agent_skill_targets`. Checkpoint 52 presents the host response.

`PreferencesAction::InstallAgentSkill` returns `DiscoverAgentSkillTargets` in `crates/skit-ui/src/preferences.rs`. `render_agent_skill_picker` renders the title through `text(locale, "Teach an AI agent to use skit")` in `crates/skit-tui/src/screens/preferences.rs`. Its Simplified and Traditional Chinese catalog values in `crates/skit-i18n/src/lib.rs` retain `agent` in English.

This is reachable from Preferences with Ctrl+K or by clicking the visible agent-skill control. It breaks the required complete Simplified and Traditional Chinese localization. Translate that catalog row with the same term used by the command label: `代理` in Simplified Chinese and `代理` in Traditional Chinese.

## Review method

Read `README.md` and `manifest.json` first. Reviewed all 32 manifest chunks in manifest order and all 724 checkpoint rows: 181 rows for each of `en-80x24`, `zh-cn-120x30`, `zh-tw-40x40`, and `pseudo-120x12`.

Parsed every first referenced reducer, host, session, styled-frame, and geometry object. This covered 1,576 unique objects: 199 reducer, 428 host, 484 session, 385 styled-frame, and 80 geometry objects. Used structured row deltas and readable views to check canonical state, host effects, input resolution, hit geometry, and displayed frames. The review included 552 presented rows and 172 `not_presented` rows. The latter supplied causal state only and were not treated as visible frames.

Checked keyboard and mouse paths, footer affordances, layout at all four viewport sizes, and localized visible text. Traced the finding through the current reducer action, host effect, TUI renderer, and i18n catalog. Each final-liveness checkpoint reports `passed`.

The corpus covers one shared 100-operation vector and its final-liveness step. This review did not run additional live terminal interactions outside that vector.
