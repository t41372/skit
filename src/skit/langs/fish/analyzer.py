"""fish analyzer: env-default candidate detection via a hand scanner (see the package docstring).

fish syntax is regular — no heredocs, simple quoting — so a small quote-aware line scanner is
enough; there is no maintained tree-sitter-fish PyPI wheel to lean on. The scanner is **total**:
an ambiguous or malformed line is skipped silently (a missed hint is cheaper than a wrong one),
so there is no `syntax_error` concept — `analyze` always returns a well-formed `Analysis`.

**v1 emits only env-default candidates** — the ``set -q NAME; or set NAME value`` idiom. fish
sees inherited environment variables as ordinary variables, so env delivery is native (zero
rewrite, no injector). const/read detection is deferred (their delivery needs an injector fish
does not have yet), so they are not emitted — emitting a candidate skit cannot deliver would be
dishonest.

The env-default rules mirror the shell analyzer's:

- The idiom is two adjacent top-level statements: a query ``set -q NAME`` followed by a
  conditional assignment ``or set NAME value`` (the ``or`` only fires when NAME is unset, so it
  preserves an inherited value — that is why it is env-deliverable). The two may sit on one line
  (``set -q NAME; or set NAME x``) or across a newline (fish continues an ``or`` at line start).
- **Suppression (the #1 correctness rule):** a plain, unconditional top-level ``set NAME value``
  anywhere clobbers the inherited value, so an env overlay would silently no-op — that NAME is
  dropped. The idiom's own guarded ``or set`` half is conditional, so it never self-suppresses.
- **Top-level only:** statements inside function/if/while/for/begin/switch blocks are ignored
  (a block-depth counter tracks the keywords; `end` decrements).
- Types int/float by the same shape as shell (``-?\\d+`` / ``-?\\d+\\.\\d+``); never bool.
  Leading-underscore names are skipped; first occurrence wins.

Hints (free, harmless over-detection): ``$argv`` ⇒ uses_argv; ``(status filename)`` /
``(status dirname)`` ⇒ uses_self_location.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import TYPE_CHECKING

from ... import analysis
from ...analysis import Analysis, Candidate
from ...params import is_secret_name

if TYPE_CHECKING:
    from ...analysis import Report
    from ...params import ParamDecl

# Block-opening keywords (each closed by `end`); a statement whose head is one of these opens a
# block, so everything after it (until the matching `end`) is nested, not top-level.
_OPENERS = frozenset({"function", "if", "while", "for", "begin", "switch"})

# Leading job-conjunction keywords: `and`/`or`/`not` prefix a conditional statement, which never
# unconditionally clobbers a variable (so it can't be a suppressor) and is the idiom's `or set` half.
_CONDITIONAL_PREFIXES = frozenset({"and", "or", "not"})

_INT_RE = re.compile(r"-?\d+")
_FLOAT_RE = re.compile(r"-?\d+\.\d+")


def analyze(text: str) -> Analysis:
    """Detect env-default candidates in a fish script (v1: env idiom only). Total — any input
    yields an Analysis without raising, so `add` can always take the script into the store."""
    stmts = _statements_with_depth(text)
    envdefaults = _envdefault_candidates(stmts)
    clobbered = _clobbered_names(stmts)
    candidates = [c for c in envdefaults if c.name not in clobbered]
    code = _code(text)
    return Analysis(
        candidates=candidates,
        uses_argv="$argv" in code,
        uses_self_location=_has_self_location(code),
    )


def reconcile(text: str, specs: list[ParamDecl]) -> Report:
    """Reconcile the [tool.skit] definitions with the fish script (wires the fish analyzer into
    the neutral reconcile). envdefault drift is matched by name, exactly like shell's."""
    return analysis.reconcile(text, specs, analyze=analyze)


# ---------------------------------------------------------------- lines & tokenizing


def _logical_lines(text: str) -> list[tuple[str, int]]:
    """(logical line, 1-based start line number) with trailing-backslash continuations joined.
    An odd count of trailing backslashes continues onto the next physical line (the last one is
    the continuation marker); an even count is literal escaped backslashes."""
    out: list[tuple[str, int]] = []
    pending = ""
    pending_lineno = 0  # pragma: no mutate — reassigned to `idx` on the first iteration (pending is "" at start) before any read, so this initializer's value (0 / None / 1) is unobservable
    for idx, physical in enumerate(text.split("\n"), start=1):
        if not pending:
            pending_lineno = idx
        combined = pending + physical
        trailing = len(combined) - len(combined.rstrip("\\"))  # pragma: no mutate — `trailing` is read only as `trailing % 2`; `len + len(rstrip)` differs from `len - len(rstrip)` by 2·len(rstrip) (even), so the -/+ swap keeps the same parity and is unobservable; the killable rstrip-arg mutation stays pinned by the continuation tests  # fmt: skip
        if trailing % 2 == 1:
            pending = combined[:-1]  # drop the continuation backslash; join with the next line
        else:
            out.append((combined, pending_lineno))
            pending = ""
    if pending:
        out.append((pending, pending_lineno))
    return out


def _tokenize(line: str) -> list[str]:
    """Quote-aware word tokens for one logical line; a bare `;` is its own token, and an unquoted
    `#` at a word boundary ends the line (comment). Single quotes escape only ``\\'`` and ``\\\\``;
    double quotes may contain `$` (kept literal for scanning). Total: an unterminated quote just
    consumes to end of line."""
    tokens: list[str] = []
    cur = ""
    i = 0
    n = len(line)
    while i < n:
        ch = line[i]
        if ch in ("'", '"'):
            cur += ch
            i += 1
            while i < n:
                c = line[i]
                cur += c
                if c == "\\" and i + 1 < n:  # pragma: no mutate — `i + 2 < n` is a true equivalent here (a stricter bound only skips differently when the escaped char is the line's last, where the scan already ends with an identical token); this line pragma also suppresses the killable `c == "\\"` / `i - 1 < n` / `i + 1 <= n` variants, which are separately pinned by the tokenize escape tests  # fmt: skip
                    cur += line[i + 1]
                    i += 2
                    continue
                i += 1
                if c == ch:
                    break
            continue
        if ch == "#" and not cur:
            break  # a `#` at a word start is a comment to end of line
        if ch.isspace():
            if cur:
                tokens.append(cur)
                cur = ""
            i += 1
            continue
        if ch == ";":
            if cur:
                tokens.append(cur)
                cur = ""
            tokens.append(";")
            i += 1
            continue
        if ch == "\\" and i + 1 < n:
            cur += ch + line[i + 1]
            i += 2
            continue
        cur += ch
        i += 1
    if cur:
        tokens.append(cur)
    return tokens


def _statements(text: str) -> list[tuple[list[str], int]]:
    """Every statement (a `;`/newline-separated run of word tokens) with its start line number,
    in source order across the whole file. Empty statements are dropped."""
    out: list[tuple[list[str], int]] = []
    for line, lineno in _logical_lines(text):
        current: list[str] = []
        for tok in _tokenize(line):
            if tok == ";":
                if current:
                    out.append((current, lineno))
                    current = []
            else:
                current.append(tok)
        if current:
            out.append((current, lineno))
    return out


def _statements_with_depth(text: str) -> list[tuple[list[str], int, int]]:
    """Every statement annotated with its block depth (0 = top level). A block-opener statement
    is recorded at the OUTER depth (its header sits at the enclosing level), then subsequent
    statements are one deeper until the matching `end`; `end` decrements (clamped at 0, so a
    stray `end` can never drive the depth negative)."""
    out: list[tuple[list[str], int, int]] = []
    depth = 0
    for words, lineno in _statements(text):
        head = words[0]
        if head == "end":
            depth = max(0, depth - 1)
            continue
        out.append((words, depth, lineno))
        if head in _OPENERS:
            depth += 1
    return out


def _dequote(word: str) -> str:
    """Best-effort literal text of a fish word (concatenated bare / single- / double-quoted
    segments). Single quotes process only ``\\'`` and ``\\\\``; double quotes also unescape ``$``.
    Used for the env-default's displayed default and for argparse spec strings."""
    out = ""
    i = 0
    n = len(word)
    while i < n:
        ch = word[i]
        if ch == "'":
            i += 1
            while i < n and word[i] != "'":
                if word[i] == "\\" and i + 1 < n and word[i + 1] in ("'", "\\"):
                    out += word[i + 1]
                    i += 2
                    continue
                out += word[i]
                i += 1
            i += 1  # skip the closing quote (or run off the end on an unterminated quote)
        elif ch == '"':
            i += 1
            while i < n and word[i] != '"':
                if word[i] == "\\" and i + 1 < n and word[i + 1] in ('"', "\\", "$"):
                    out += word[i + 1]
                    i += 2
                    continue
                out += word[i]
                i += 1
            i += 1
        elif ch == "\\" and i + 1 < n:
            out += word[i + 1]
            i += 2
        else:
            out += ch
            i += 1
    return out


def _infer(value: str) -> tuple[str, str | int | float]:
    """(type, typed default) for a literal's text — int/float by shape, str otherwise; never bool
    (mirrors the shell analyzer)."""
    if _INT_RE.fullmatch(value):
        return "int", int(value)
    if _FLOAT_RE.fullmatch(value):
        return "float", float(value)
    return "str", value


# ---------------------------------------------------------------- set-statement classification


@dataclass(frozen=True)
class _SetStmt:
    """A parsed `set` statement (or None when the statement isn't a `set`)."""

    conditional: bool  # prefixed by and/or/not — a guarded run, never an unconditional clobber
    is_query: bool  # carries -q/--query — a test, not an assignment
    name: str | None  # the target variable (first operand), or None
    value: list[str]  # the value tokens after the name (raw, still quoted)


def _classify_set(words: list[str]) -> _SetStmt | None:
    """Classify a statement as a `set` (after stripping leading and/or/not), or None."""
    j = 0
    conditional = False
    while j < len(words) and words[j] in _CONDITIONAL_PREFIXES:
        conditional = True
        j += 1
    rest = words[j:]
    if not rest or rest[0] != "set":
        return None
    flags: list[str] = []
    operands: list[str] = []
    options_done = False  # pragma: no mutate — =None is a true equivalent (read only via `not options_done`); this line pragma also suppresses the killable =True, which is separately pinned by test_classify_set_matrix
    for w in rest[1:]:
        if not options_done and w == "--":
            options_done = True
        elif not options_done and not operands and w.startswith("-"):
            flags.append(w)
        else:
            operands.append(w)
    return _SetStmt(
        conditional=conditional,
        is_query=_is_query(flags),
        name=operands[0] if operands else None,
        value=operands[1:],
    )


def _is_query(flags: list[str]) -> bool:
    """Whether a `set`'s flags include -q/--query (also inside a short cluster like -gq)."""
    for f in flags:
        if f == "--query":
            return True
        if not f.startswith("--") and "q" in f[1:]:
            return True
    return False


# ---------------------------------------------------------------- env-default


def _envdefault_candidates(stmts: list[tuple[list[str], int, int]]) -> list[Candidate]:
    """Every ``set -q NAME`` (top level) immediately followed by a conditional ``or set NAME
    value`` (top level, same name, non-empty value) — one candidate per name, first occurrence's
    default. Leading-underscore names are skipped, like the shell/python analyzers."""
    parsed = [(_classify_set(words), depth, lineno) for words, depth, lineno in stmts]
    out: list[Candidate] = []
    seen: set[str] = set()
    for i in range(len(parsed) - 1):
        st, depth, lineno = parsed[i]
        nxt, nxt_depth, _ = parsed[i + 1]
        if st is None or depth != 0 or not st.is_query or st.name is None:
            continue
        if nxt is None or nxt_depth != 0 or nxt.is_query or not nxt.conditional:
            continue
        if nxt.name != st.name or not nxt.value:
            continue
        name = st.name
        if name.startswith("_") or name in seen:
            continue
        seen.add(name)
        type_name, default = _infer(" ".join(_dequote(v) for v in nxt.value))
        out.append(
            Candidate(
                binding="envdefault",
                name=name,
                env_name=name,
                type=type_name,
                default=default,
                lineno=lineno,
                secret=is_secret_name(name),
            )
        )
    return out


def _clobbered_names(stmts: list[tuple[list[str], int, int]]) -> set[str]:
    """Names a plain, unconditional top-level `set NAME …` assigns — an env overlay for one of
    these would be silently overwritten, so its env-default is suppressed. A `-q` query and a
    conditional (and/or/not-prefixed, incl. the idiom's own `or set`) never clobber."""
    out: set[str] = set()
    for words, depth, _lineno in stmts:
        if depth != 0:
            continue
        st = _classify_set(words)
        if st is None or st.is_query or st.conditional or st.name is None:
            continue
        out.add(st.name)
    return out


# ---------------------------------------------------------------- hints


def _code(text: str) -> str:
    """The script with comments stripped, for cheap substring hint checks."""
    return "\n".join(_strip_comment(line) for line, _ in _logical_lines(text))


def _strip_comment(line: str) -> str:
    """`line` with a trailing unquoted `#`-comment removed (quote-aware). Total."""
    i = 0
    n = len(line)
    quote: str | None = None
    while i < n:
        ch = line[i]
        if quote is not None:
            if ch == "\\" and i + 1 < n:  # pragma: no mutate — inside a quote this branch only decides how far to skip; every `i + 1 < n` variant (i-1<n / i+2<n / i+1<=n) differs from it solely at the line's last char, where the loop already exits with the same return, so all are equivalent; the killable `ch == "\\"` string mutation stays pinned by test_strip_comment_paths  # fmt: skip
                i += 2
                continue
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in ("'", '"'):
            quote = ch
            i += 1
            continue
        if ch == "\\" and i + 1 < n:  # pragma: no mutate — the escaped-skip's exact bound is immaterial (a total scanner tolerates an over/under-skip only at the line's last char, where it already exits with the same return), so i-1<n / i+2<n / i+1<=n are all equivalent; the killable `ch == "\\"` string mutation stays pinned by test_strip_comment_backslash_escapes_a_quote_outside_quotes  # fmt: skip
            i += 2
            continue
        if ch == "#" and (i == 0 or line[i - 1].isspace()):
            return line[:i]
        i += 1
    return line


def _has_self_location(code: str) -> bool:
    """``(status filename)`` / ``(status dirname)`` — the script asks where it lives."""
    return "status filename" in code or "status dirname" in code
