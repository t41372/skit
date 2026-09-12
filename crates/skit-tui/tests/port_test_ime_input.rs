//! Mechanical port of the Python oracle module `tests/test_ime_input.py`
//! (`origin/main@206f9ef`): the CJK IME regression guard (the "cannot type Chinese in
//! iTerm2" incident). Each `#[test]` keeps its Python `def test_*` name so it traces back
//! to its origin, and each Python "WHY" comment is preserved above it.
//!
//! What the oracle guards, and why the Rust rewrite maps to all-`#[ignore]` stubs:
//!
//! - The incident: Textual >= 8.2.7 pushes the kitty keyboard protocol in "report all
//!   keys as escape codes" mode, which iTerm2 3.6.x implements in a way that fights the
//!   macOS IME — the candidate-selection digit and the commit Enter arrive as raw key
//!   events while the composed CJK text is never delivered. No skit binding needs the
//!   protocol, so the Python package opts out at import time with
//!   `os.environ.setdefault("TEXTUAL_DISABLE_KITTY_KEY", "1")` (`src/skit/__init__.py:15`).
//!   All three tests observe that one Textual-framework mechanism: the env var is set at
//!   package import (test 1), an explicit user `=0` override survives (test 2), and
//!   `textual.constants.DISABLE_KITTY_KEY` actually reads `True` end to end (test 3).
//!
//! - The Rust rewrite runs on Ratatui + Crossterm, not Textual. Crossterm activates the
//!   kitty "report all keys" protocol ONLY through an explicit
//!   `PushKeyboardEnhancementFlags`; without that call the protocol is never on, so the
//!   IME works. skit's terminal setup pushes only raw mode + the alternate screen + mouse
//!   capture and never touches keyboard-enhancement flags (`crates/skit-tui/src/terminal.rs`
//!   lines 114-115, 286-287, 365-368 are the only terminal-mode sites). A grep of the whole
//!   `crates/` tree finds no `PushKeyboardEnhancementFlags`, and neither backend dependency
//!   (`ratatui-crossterm 0.1.2`, `ratatui-interact 0.5.3`) pushes the flags on skit's
//!   behalf. The oracle's contract — "the kitty report-all-keys protocol must not be active
//!   in skit's TUI" — is therefore satisfied BY CONSTRUCTION, not missing.
//!
//! - There is no `TEXTUAL_DISABLE_KITTY_KEY` env var, no import side effect, and no
//!   `textual.constants` in the Rust surface, so none of the three assertions has a public
//!   API to drive. The only observable that survives the stack change is "which escape
//!   sequences the composed binary emits to the terminal", and that lives at the
//!   real-terminal / PTY tier of `skit-cli-rs`, unreachable from a `skit-tui` integration
//!   test without a PTY harness (a forbidden `Cargo.toml` / dependency edit). This matches
//!   the in-series precedent that other Textual/Crossterm key-routing mechanics are stubbed
//!   `CROSS-CRATE` (see `port_test_prompt_tui.rs:1230`).
//!
//! Buckets:
//! - REAL asserting tests: NONE. This module is 100% Textual-framework-internal opt-out.
//! - CROSS-CRATE (`#[ignore]` stubs, all three): the observable is a terminal-emitted
//!   escape sequence at the real-terminal / PTY tier of the composed binary. Each stub
//!   records the Python behavior and the satisfied-by-construction evidence.
//!
//! NOTE (regression guard): the Rust tree has no positive test that would catch a future
//! `PushKeyboardEnhancementFlags` slipping into `terminal.rs` (or any `execute!` site). The
//! oracle's guard is Textual-specific and not portable without a PTY harness. Whether to add
//! a PTY-tier guard is a main-agent / ledger decision, not something to invent here. A
//! single-file source scan of `terminal.rs` was rejected: it drives no public API and would
//! false-green if the flags were pushed from another module.
//!
//! An all-stub file must carry no `use` imports — an unused import fails clippy `-D warnings`.

/// WHY (oracle): importing skit must set `TEXTUAL_DISABLE_KITTY_KEY=1` so Textual opts out
/// of the kitty keyboard protocol before it reads the flag at import time.
/// `assert os.environ["TEXTUAL_DISABLE_KITTY_KEY"] == "1"` (`tests/test_ime_input.py:21`,
/// impl `src/skit/__init__.py:15`).
///
/// CROSS-CRATE (real-terminal / PTY tier of `skit-cli-rs`): there is no
/// `TEXTUAL_DISABLE_KITTY_KEY` env var and no package-import side effect in the Rust stack.
/// The equivalent contract — the kitty "report all keys" protocol stays off — is satisfied
/// by construction: Crossterm enables it only via `PushKeyboardEnhancementFlags`, which
/// skit's terminal setup never calls (`crates/skit-tui/src/terminal.rs:114-115,286-287`).
/// No `skit-tui` integration test can observe the emitted escape sequences.
#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs real-terminal / PTY tier): no TEXTUAL_DISABLE_KITTY_KEY in the Rust stack; the kitty protocol stays off by construction (Crossterm needs an explicit PushKeyboardEnhancementFlags, which terminal.rs never calls). tests/test_ime_input.py:21"]
fn test_kitty_protocol_opt_out_is_set_at_package_import() {}

/// WHY (oracle): setdefault, not assignment — a user's explicit `TEXTUAL_DISABLE_KITTY_KEY=0`
/// must survive the import so they can re-enable the protocol on a terminal whose kitty
/// implementation coexists with their IME. `assert os.environ[...] == "0"`
/// (`tests/test_ime_input.py:27`, impl `src/skit/__init__.py:15`).
///
/// CROSS-CRATE + override adjudication: `TEXTUAL_DISABLE_KITTY_KEY` is Textual's own
/// namespace, not a skit feature. skit only refrained from clobbering a user value that
/// Textual would read. With Textual gone, there is no such env var and nothing to re-enable
/// (the protocol is never enabled to begin with), so preserving the override is vacuous and
/// there is NO feature loss under the superset rule — skit added a launcher on a stack that
/// never needed the opt-out, it did not drop a behavior.
#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs real-terminal / PTY tier): TEXTUAL_DISABLE_KITTY_KEY is Textual's namespace, absent from the Rust stack; with the protocol never enabled there is nothing to re-enable, so the setdefault override is vacuous and no feature is lost. tests/test_ime_input.py:27"]
fn test_kitty_protocol_opt_out_respects_an_explicit_user_override() {}

/// WHY (oracle): end-to-end wiring — in any process that imports skit first, Textual must
/// actually see the opt-out, so `textual.constants.DISABLE_KITTY_KEY is True`. Guards against
/// the flag moving somewhere that loads after `textual.constants`
/// (`tests/test_ime_input.py:35`, impl `src/skit/__init__.py:15`).
///
/// CROSS-CRATE (real-terminal / PTY tier of `skit-cli-rs`): there is no `textual.constants`
/// to read in the Rust stack. The end-to-end equivalent — the composed binary never emits
/// the kitty enhancement-flag escape sequence — is observable only against a real terminal /
/// PTY, not from a `skit-tui` integration test. Grep confirms no `PushKeyboardEnhancementFlags`
/// anywhere in `crates/`, and neither `ratatui-crossterm` nor `ratatui-interact` pushes the
/// flags, so the protocol is off end to end.
#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs real-terminal / PTY tier): no textual.constants in the Rust stack; the end-to-end 'protocol stays off' is observable only against a real terminal/PTY, and grep confirms no PushKeyboardEnhancementFlags in crates/ or the backend deps. tests/test_ime_input.py:35"]
fn test_kitty_protocol_opt_out_lands_before_textual_reads_it() {}
