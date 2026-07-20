"""Shell analyzer: candidate-parameter detection for bash/sh/zsh via tree-sitter-bash.

skit's flagship non-Python capability. It mirrors the Python analyzer's structure and shares its
neutral result model (`skit.analysis.Candidate`/`Analysis`) and reconcile machinery, so add-time
detection and drift reconciliation stay language-parallel. Detection is a **node walk** over the
parse tree (no query strings — the mutmut gate would otherwise have nothing but opaque S-expression
literals to mutate), parsing `text.encode("utf-8")` so tree-sitter's absolute byte offsets line up
with the source.

Detection (docs/design/multilang.md, §"Shell analyzer/shim"):

- **const**: a top-level `variable_assignment` (direct child of `program`, or inside a
  `declaration_command` for export/readonly/declare/typeset — never `local`) with a *literal* RHS
  (`word`/`number` with no expansion, `raw_string`, or a `string` of only `string_content`).
  Empty/array/concatenation/expansion/command-substitution RHS excluded; `+=` is an accumulator, not
  a const. readonly / `declare -r` consts are excluded (no delivery can safely reassign a readonly).
  Last-write-wins dedupe (first slot, last value), leading-underscore names skipped — like Python.
- **envdefault**: a `${NAME:-default}` / `:=` / `-` / `=` expansion (env delivery, zero rewrite).
  **Suppressed** when NAME is also bare-assigned at top level (const wins; env delivery would
  silently no-op — the #1 correctness rule). One candidate per name (first occurrence's default).
- **read**: an interactive `read` (also `builtin read` / `command read`); `-p` prompt (incl.
  clustered `-sp`), `-s` ⇒ secret with certainty. **Excluded as data-reading** when fed by a pipe,
  a redirect/here-string on the read or its enclosing loop — those consume data, not a user prompt.
- **demotions**: `+=`, arithmetic self-reference (`VAR=$((VAR+1))`), `((VAR++))`/`((VAR+=1))`, `let`,
  loop-body reassignment ⇒ demoted "accumulator" (a working variable, not a parameter).
- **hints**: `$0`/`$BASH_SOURCE` ⇒ `uses_self_location`; `$1`/`$@`/`$#`/`getopts`/`shift` ⇒
  `uses_argv`.
- **types**: int iff `^-?\\d+$`, float iff `^-?\\d+\\.\\d+$`, else str — **never bool** (shell has no
  boolean type; a value like `true` is just the string "true"). "007" reads as int 7 (leading zeros
  are not preserved — a deliberate, documented call). The injectable domain is str/int/float.
- **degradation honesty**: any `tree.root_node.has_error` ⇒ `Analysis(syntax_error=True)` (empty),
  so a zsh-ism tree-sitter-bash can't parse degrades honestly to Tier-0 rather than guessing.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import TYPE_CHECKING

import tree_sitter_bash
from tree_sitter import Language, Parser

from ... import analysis
from ...analysis import Analysis, Candidate
from ...params import is_secret_name

if TYPE_CHECKING:
    from collections.abc import Iterator

    from tree_sitter import Node

    from ...analysis import Report
    from ...params import ParamDecl

_LANGUAGE = Language(tree_sitter_bash.language())

# The operator tokens that make an expansion an env-default (value comes from ${NAME} with a
# fallback). `:?`/`#`/`%`/`/` and friends are not defaults, so they never become candidates.
_ENVDEFAULT_OPERATORS = (":-", ":=", "-", "=")

# read flags that consume a following value (so their value argument is never a varname).
_READ_VALUE_FLAGS = frozenset("adiNntpu")

# read flags that REFRAME the input — the read stops early or on a different delimiter, so the one
# line skit feeds it is not the value the script ends up with (`read -n 3 X` on "abcdefgh" gives
# "abc"; `-N 5` gives ""; `-d :` stops at the colon). A managed value could not be delivered
# faithfully, so a read carrying one of these is not offered as a candidate at all — the honest
# Tier-0 degradation `read -a` already gets.
_READ_REFRAMING_FLAGS = frozenset("nNd")

# Arithmetic assignment operators: an `((...))` binary_expression with one of these on the left is a
# mutation, so that name is a working variable (accumulator), not a constant.
_ARITH_ASSIGN_OPS = frozenset(
    {"=", "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", "&=", "|=", "^="}
)

_LOOP_TYPES = frozenset({"for_statement", "while_statement", "c_style_for_statement"})

_INT_RE = re.compile(r"-?\d+")
_FLOAT_RE = re.compile(r"-?\d+\.\d+")
# A `let`/arithmetic argument token whose leading identifier is being assigned or in/decremented.
_LET_TARGET_RE = re.compile(r"([A-Za-z_]\w*)\s*(\+\+|--|<<=|>>=|[-+*/%^&|]?=)")


def analyze(text: str) -> Analysis:
    """Detect candidate parameters in a shell script. On any parse error, return an empty result
    (no exception; add can still take the script into the store — honest Tier-0 degradation)."""
    parser = Parser(_LANGUAGE)
    root = parser.parse(text.encode("utf-8")).root_node  # pragma: no mutate  (codec alias)
    if root.has_error:
        return Analysis(syntax_error=True)
    consts = _const_candidates(root)
    mutated = _mutated_names(root)
    for c in consts:
        if c.name in mutated:
            c.demoted = True
            c.demotion = "accumulator"
    bare = _bare_assigned_names(root)
    candidates = consts + _envdefault_candidates(root, bare) + _read_candidates(root)
    # The script drives its own CLI: report it, like python does — the guards that stop
    # add/params from offering to manage constants (which would REPLACE the getopts
    # form) all key on uses_cli_framework, and a python-only signal left them dead here.
    from . import cli_reader

    frameworks = ["getopts"] if cli_reader.read_cli(text) is not None else []
    return Analysis(
        candidates=candidates,
        frameworks=frameworks,
        uses_argv=_uses_argv(root),
        uses_self_location=_uses_self_location(root),
    )


def reconcile(text: str, specs: list[ParamDecl]) -> Report:
    """Reconcile the [tool.skit] definitions with the shell script's current content (wires the
    shell analyzer into the neutral reconcile — see skit.analysis.reconcile)."""
    return analysis.reconcile(text, specs, analyze=analyze)


# ---------------------------------------------------------------- tree walking


def _walk(node: Node) -> Iterator[Node]:
    """Pre-order (source-order) traversal of every named node. A hand-rolled stack rather than a
    query cursor: the mutmut gate needs real code to mutate, not an opaque S-expression string."""
    stack = [node]
    while stack:
        current = stack.pop()
        yield current
        stack.extend(reversed(current.named_children))


def _text(node: Node) -> str:
    if node.text is None:  # pragma: no cover — every node from a parsed tree carries its bytes
        return ""  # pragma: no mutate — unreachable (node.text is never None on a parsed tree)
    return node.text.decode("utf-8")  # pragma: no mutate — "UTF-8" is the same codec (alias)


def _arguments(node: Node) -> list[Node]:
    """A command's `argument`-field children, in order (excludes the command name and redirects)."""
    return [
        child for i, child in enumerate(node.children) if node.field_name_for_child(i) == "argument"
    ]


# ---------------------------------------------------------------- literals & types


def _literal_text(node: Node) -> str | None:
    """The literal string a value/argument node stands for, or None when it isn't a plain literal
    (an expansion, command substitution, array, concatenation, arithmetic — anything dynamic)."""
    kind = node.type
    if kind in ("word", "number"):
        if node.named_child_count:  # pragma: no cover — a word/number RHS is always a leaf node
            return None
        return _text(node)
    if kind == "raw_string":
        return _text(node)[1:-1]  # strip the surrounding single quotes (no escape processing)
    if kind == "string":
        if all(child.type == "string_content" for child in node.named_children):
            # An all-string_content string is always a single content node, so the join separator
            # is never used (any 2+ content string carries a non-content child, failing all()).
            return "".join(_text(child) for child in node.named_children)  # pragma: no mutate
        return None
    return None


def _infer(value: str) -> tuple[str, str | int | float]:
    """(type, typed-default) for a literal's text. int/float by shape, str otherwise; never bool —
    shell has no boolean type. "007" → int 7 (leading zeros are not preserved: a documented call)."""
    if _INT_RE.fullmatch(value):
        return "int", int(value)
    if _FLOAT_RE.fullmatch(value):
        return "float", float(value)
    return "str", value


# ---------------------------------------------------------------- const


def _toplevel_assignments(root: Node) -> Iterator[tuple[Node, bool]]:
    """Yield (variable_assignment, readonly) for each top-level assignment location: direct
    children of `program`, plus the assignments inside a top-level export/readonly/declare/typeset
    `declaration_command`. `local` is skipped entirely (function scope — never a top-level const)."""
    for child in root.named_children:
        if child.type == "variable_assignment":
            yield child, False
        elif child.type == "declaration_command":
            # a declaration_command always has children, so the else "" is unreachable
            keyword = child.children[0].type if child.children else ""  # pragma: no mutate
            if keyword == "local":
                continue
            readonly = keyword == "readonly" or _has_readonly_flag(child)
            for sub in child.named_children:
                if sub.type == "variable_assignment":
                    yield sub, readonly


def _has_readonly_flag(decl: Node) -> bool:
    """Whether a declaration_command carries `-r` (readonly), e.g. `declare -r` / `typeset -rx`."""
    for child in decl.named_children:
        if child.type == "word":
            word = _text(child)
            if word.startswith("-") and not word.startswith("--") and "r" in word[1:]:
                return True
    return False


def _const_candidates(root: Node) -> list[Candidate]:
    """Top-level literal constant assignments, last-write-wins deduped by name (first slot kept,
    last value wins — exactly like the Python analyzer, so the injected value matches what the name
    actually holds once the script finishes running)."""
    out: list[Candidate] = []
    index_by_name: dict[str, int] = {}
    for node, readonly in _toplevel_assignments(root):
        if readonly:
            continue  # a readonly can't be safely reassigned by any delivery — excluded v1
        name_node = node.child_by_field_name("name")
        value_node = node.child_by_field_name("value")
        if name_node is None or name_node.type != "variable_name" or value_node is None:
            continue  # `VAR=` (no value), or a subscript/array target — not a plain const
        if _assignment_operator(node) != "=":
            continue  # `+=` is an accumulator, handled by demotion — not a literal const
        name = _text(name_node)
        if name.startswith("_"):
            continue  # conventionally private/internal values; not treated as parameters
        literal = _literal_text(value_node)
        if not literal:  # not a plain literal, or an empty value ("" / '') — excluded
            continue
        type_name, default = _infer(literal)
        candidate = Candidate(
            binding="const",
            name=name,
            type=type_name,
            default=default,
            lineno=node.start_point[0] + 1,
            secret=is_secret_name(name),
        )
        if name in index_by_name:
            out[index_by_name[name]] = candidate  # last occurrence's data wins; keep first slot
        else:
            index_by_name[name] = len(out)
            out.append(candidate)
    return out


def _bare_assigned_names(root: Node) -> set[str]:
    """Names a top-level assignment sets in a way that would *clobber* an inherited env value. An
    envdefault whose NAME is in here is suppressed — the script overwrites it, so an env value would
    be silently ignored (the #1 correctness rule).

    A *self-reading* assignment is deliberately excluded: `PORT=${PORT:-8080}` (the canonical
    env-default idiom, and what add-time normalization produces from `PORT=8080`) reads `$PORT`
    first, so it preserves the env value and IS the envdefault — it must not suppress itself. Only a
    clobbering assignment (`PORT=8080`, `PORT=$(…)`, `PORT=$OTHER`) suppresses."""
    names: set[str] = set()
    for node, _ in _toplevel_assignments(root):
        name_node = node.child_by_field_name("name")
        # name_node is never None here; or->and only differs on a subscript target, whose bracketed
        # text never matches a scalar envdefault name (inert) — hence equivalent.
        if name_node is None or name_node.type != "variable_name":  # pragma: no mutate
            continue
        name = _text(name_node)
        value = node.child_by_field_name("value")
        if value is not None and _references(value, name):
            continue  # self-reading (e.g. NAME=${NAME:-x}) preserves the env value — not a clobber
        names.add(name)
    return names


def _assignment_operator(node: Node) -> str:
    """A variable_assignment's operator token: `=` or `+=` (the grammar exposes it as an anonymous
    child, not a field)."""
    for child in node.children:
        # dropping "=" is equivalent: an `=` assignment then falls through to the "=" return below
        if child.type in ("=", "+="):  # pragma: no mutate
            return child.type
    return "="  # pragma: no cover — a variable_assignment always carries one of the two tokens # pragma: no mutate — unreachable fallback


# ---------------------------------------------------------------- envdefault


def _envdefault_candidates(root: Node, bare_assigned: set[str]) -> list[Candidate]:
    """Every `${NAME:-default}`-style expansion, one candidate per name (first occurrence's
    default), suppressed when NAME is bare-assigned at top level."""
    out: list[Candidate] = []
    seen: set[str] = set()
    for node in _walk(root):
        if node.type != "expansion":
            continue
        operator = node.child_by_field_name("operator")
        if operator is None or operator.type not in _ENVDEFAULT_OPERATORS:
            continue
        var = node.named_children[0] if node.named_children else None
        if var is None or var.type != "variable_name":
            continue  # skip a subscript target like ${ARR[0]:-x}
        name = _text(var)
        if name in seen or name in bare_assigned:
            continue
        seen.add(name)
        type_name, default = _infer(_expansion_default(node, operator))
        out.append(
            Candidate(
                binding="envdefault",
                name=name,
                env_name=name,
                type=type_name,
                default=default,
                lineno=node.start_point[0] + 1,
                secret=is_secret_name(name),
            )
        )
    return out


def _expansion_default(node: Node, operator: Node) -> str:
    """The default text of an expansion: the bytes between the operator and the closing `}`."""
    body = node.text or b""  # pragma: no mutate  (an expansion's text is never empty; b"" is dead)
    start = operator.end_byte - node.start_byte
    return body[start:-1].decode("utf-8")  # pragma: no mutate  (drops trailing }; codec alias)


# ---------------------------------------------------------------- read


@dataclass(frozen=True)
class ReadFlags:
    """The parsed shape of one `read` command — everything the injector needs to deliver a value
    faithfully, or to know that it cannot."""

    secret: bool  # -s: certainty, not a heuristic
    prompt: str  # -p's literal text ("" when dynamic or absent)
    varnames: list[str]
    raw: bool  # -r: backslashes are literal. Without it, `read` processes them (see inject._feed_value)
    reframing: (
        bool  # -n/-N/-d: the read reframes its input, so a fed line isn't the delivered value
    )


def _has_ifs_prefix(node: Node) -> bool:
    """`IFS=… read …` — a variable_assignment prefix on the read command that redefines the field
    separator.

    skit joins a multi-variable read's values with a SPACE and relies on default $IFS to split them
    back apart, and on `read`'s default edge-stripping when deciding what is safe. A custom IFS
    invalidates both halves of that model in opposite directions — `IFS=: read A B` would hand the
    whole space-joined line to A, while `IFS= read -r LINE` does no splitting or stripping at all, so
    a value skit refuses as unsafe would in fact have arrived intact. Rather than model an arbitrary
    IFS, such a read is simply not offered as a candidate (the same honest degradation `read -a` and
    the reframing flags get)."""
    return any(
        child.type == "variable_assignment"
        and (name := child.child_by_field_name("name")) is not None
        and _text(name) == "IFS"
        for child in node.children
    )


def injectable_reads(root: Node) -> list[tuple[Node, ReadFlags]]:
    """The `read` commands skit will manage, in source order — the SINGLE source of truth the
    analyzer (which numbers candidates) and the injector (which numbers rewrite sites) BOTH consume,
    so the two can never disagree on which reads count or in what order. Every divergence between
    them has been a silent-wrong-value bug (a value landing on the wrong `read`), so there is exactly
    one place that decides membership:

    - not a read, or a data-reading form (pipe/redirect/here-string/loop-fed) → excluded;
    - reframes its input (`-n`/`-N`/`-d`) or redefines `IFS` → excluded (skit cannot deliver a value
      through the one line it feeds), the same honest degradation `read -a` already gets.
    """
    reads: list[tuple[Node, ReadFlags]] = []
    for node in _walk(root):
        if node.type != "command":
            continue
        parsed = _read_flags(node)
        if parsed is None or _is_data_read(node):
            continue
        if parsed.reframing or _has_ifs_prefix(node):
            continue
        reads.append((node, parsed))
    reads.sort(key=lambda pair: pair[0].start_byte)
    return reads


def _read_candidates(root: Node) -> list[Candidate]:
    """Every interactive `read` varname, numbered by source order (B1). Membership and order come
    from the shared `injectable_reads` so they match the injector's rewrite sites exactly."""
    out: list[Candidate] = []
    order = 0
    for node, flags in injectable_reads(root):
        secret, prompt, varnames = flags.secret, flags.prompt, flags.varnames
        for varname in varnames:
            candidate = Candidate(
                binding="input",
                name=f"input-{order + 1}",
                prompt=prompt,
                order=order,
                lineno=node.start_point[0] + 1,
                secret=secret or is_secret_name(prompt) or is_secret_name(varname),
            )
            candidate.type = "str"
            out.append(candidate)
            order += 1
    return out


def _read_flags(node: Node) -> ReadFlags | None:
    """The parsed shape of a `read` command, or None when the command isn't a read.
    Handles `builtin read` / `command read`, clustered flags (`-sp`), value-consuming flags, and a
    `--` end-of-options marker; a dynamic (non-literal) prompt collapses to ""."""
    name_node = node.child_by_field_name("name")
    if name_node is None:
        return None  # pragma: no cover — a `command` always has a command_name
    command = _text(name_node)
    args = _arguments(node)
    if command == "read":
        read_args = args
    elif command in ("builtin", "command") and args and _text(args[0]) == "read":
        read_args = args[1:]  # `builtin read …` / `command read …`
    else:
        return None
    return _parse_read_args(read_args)


def _parse_read_args(args: list[Node]) -> ReadFlags:
    secret = False
    prompt = ""
    varnames: list[str] = []
    raw = False
    reframing = False
    # only read via `not options_done`, so None is falsy-identical to False (equivalent)
    options_done = False  # pragma: no mutate
    i = 0
    while i < len(args):
        arg = args[i]
        word = _text(arg)
        is_flag = arg.type == "word" and word.startswith("-") and len(word) > 1
        if not options_done and arg.type == "word" and word == "--":
            options_done = True
        elif not options_done and is_flag:
            cluster = word[1:]
            secret_here, prompt_here, consumes_next, raw_here, reframing_here = _scan_read_cluster(
                cluster, args, i
            )
            secret = secret or secret_here
            prompt = prompt_here if prompt_here else prompt
            raw = raw or raw_here
            reframing = reframing or reframing_here
            if consumes_next:
                i += 1
        elif arg.type == "word":
            varnames.append(word)  # a plain word after the flags is a target variable name
        i += 1
    return ReadFlags(secret=secret, prompt=prompt, varnames=varnames, raw=raw, reframing=reframing)


def _scan_read_cluster(
    cluster: str, args: list[Node], i: int
) -> tuple[bool, str, bool, bool, bool]:
    """Read one flag cluster (`-s`, `-sp`, `-n5`, …):
    (secret_seen, prompt_text, consumes_next, raw_seen, reframing_seen)."""
    secret = False
    # both are only consumed via truthiness downstream, so None is falsy-identical (equivalent)
    prompt = ""  # pragma: no mutate
    consumes_next = False  # pragma: no mutate
    raw = False
    reframing = False
    j = 0
    while j < len(cluster):
        ch = cluster[j]
        # raw/reframing are decided per OPTION LETTER, inside the same walk that stops at the first
        # value-consuming flag — never over the whole cluster string, which would read letters out of
        # an attached VALUE (`-pSure?` has an 'r'; `-pEnter` an 'n') and wrongly flag the read.
        if ch == "r":
            raw = True
            j += 1
            continue
        if ch == "s":
            secret = True
            j += 1
            continue
        if ch in _READ_VALUE_FLAGS:
            # n/N/d reframe the input; they are members of _READ_VALUE_FLAGS, so they land here and
            # set reframing at exactly the point the cluster stops being option letters.
            if ch in _READ_REFRAMING_FLAGS:
                reframing = True
            attached = cluster[j + 1 :]
            if ch == "p":
                if attached:
                    prompt = attached  # `-pPROMPT` — the prompt is attached to the flag
                elif i + 1 < len(args):
                    prompt = _literal_argument(args[i + 1])  # `-p PROMPT` — next arg is the prompt
                    consumes_next = True
            # the boundary only diverges for a trailing value-flag, whose spurious consume has no
            # following arg to skip (inert)
            elif not attached and i + 1 < len(args):  # pragma: no mutate
                consumes_next = True  # `-t 5`, `-n 3` … — skip the value so it's not a varname
            break  # the rest of the cluster is this flag's attached value (or nothing)
        j += 1  # an unknown / no-value flag letter (e, a, …) — keep scanning the cluster
    return secret, prompt, consumes_next, raw, reframing


def _literal_argument(node: Node) -> str:
    """A read prompt's literal text, or "" when it's dynamic (an expansion / command sub)."""
    literal = _literal_text(node)
    return literal if literal is not None else ""


def _is_data_read(command: Node) -> bool:
    """Whether a `read` consumes data rather than prompting a user: a non-first pipeline operand, or
    a command/loop whose enclosing `redirected_statement` feeds stdin (file `<`, heredoc,
    here-string). `while read x; do …; done < f` and `cmd | while read x` are both data reads."""
    if _feeds_stdin(command):  # a here-string redirect hangs directly off the command node
        return True
    node = command
    parent = node.parent
    while parent is not None:
        if _nonfirst_pipe_operand(parent, node):
            return True
        if _redirect_feeds_body(parent, node):
            return True
        node = parent
        parent = parent.parent
    return False


def _nonfirst_pipe_operand(parent: Node, node: Node) -> bool:
    """`node` is a pipeline operand other than the first (so it's fed by the pipe, not a terminal)."""
    if parent.type != "pipeline":
        return False
    children = parent.named_children
    return bool(children) and children[0].id != node.id


def _redirect_feeds_body(parent: Node, node: Node) -> bool:
    """`node` is the body of a `redirected_statement` whose redirect feeds stdin."""
    if parent.type != "redirected_statement":
        return False
    body = parent.child_by_field_name("body")
    return body is not None and body.id == node.id and _feeds_stdin(parent)


def _feeds_stdin(redirected: Node) -> bool:
    for child in redirected.named_children:
        if child.type in ("heredoc_redirect", "herestring_redirect"):
            return True
        if child.type == "file_redirect" and _redirect_is_stdin(child):
            return True
    return False


def _redirect_is_stdin(file_redirect: Node) -> bool:
    """A `file_redirect` whose operator reads stdin (`<`), not `>`/`>>`/`2>`."""
    operator = file_redirect.child(0)
    return operator is not None and operator.type == "<"


# ---------------------------------------------------------------- demotions


def _mutated_names(root: Node) -> set[str]:
    """Names that behave like working variables, not parameters: `+=`, arithmetic self-reference,
    `((VAR++))`/`((VAR+=1))`, `let` targets, or any reassignment inside a for/while loop body."""
    out: set[str] = set()
    for node in _walk(root):
        kind = node.type
        if kind == "variable_assignment":
            _collect_assignment_mutation(node, out)
        elif kind == "postfix_expression":
            name = _first_variable_name(node)
            if name is not None:  # pragma: no branch — a postfix `x++` always wraps a variable_name
                out.add(name)
        elif kind == "binary_expression":
            _collect_arithmetic_assignment(node, out)
        elif kind == "command":
            _collect_let_targets(node, out)
        elif kind in _LOOP_TYPES:
            _collect_loop_reassignments(node, out)
    return out


def _collect_assignment_mutation(node: Node, out: set[str]) -> None:
    name_node = node.child_by_field_name("name")
    # name_node is never None here; or->and only differs on a subscript target, whose bracketed
    # text never matches a scalar candidate name (inert) — hence equivalent.
    if name_node is None or name_node.type != "variable_name":  # pragma: no mutate
        return
    name = _text(name_node)
    if _assignment_operator(node) == "+=":
        out.add(name)
        return
    value = node.child_by_field_name("value")
    if value is not None and _references(value, name):
        out.add(name)  # self-reference like VAR=$((VAR+1)) — an accumulator


def _collect_arithmetic_assignment(node: Node, out: set[str]) -> None:
    if _binary_operator(node) in _ARITH_ASSIGN_OPS:
        left = node.child_by_field_name("left")
        # left is never None here; and->or only differs on a subscript target, whose bracketed
        # text never matches a scalar candidate name (inert) — hence equivalent.
        if left is not None and left.type == "variable_name":  # pragma: no mutate
            out.add(_text(left))


def _collect_let_targets(node: Node, out: set[str]) -> None:
    name_node = node.child_by_field_name("name")
    if name_node is None or _text(name_node) != "let":
        return
    for arg in _arguments(node):
        target = _let_target(_text(arg))
        if target is not None:
            out.add(target)


def _collect_loop_reassignments(node: Node, out: set[str]) -> None:
    for sub in _walk(node):
        if sub.type == "variable_assignment":
            name_node = sub.child_by_field_name("name")
            # name_node is never None here; and->or is equivalent (a subscript target's bracketed
            # text never matches a scalar candidate name). The type-check mutations on this line are
            # pinned behaviourally by test_plain_loop_body_reassignment_* instead.
            if name_node is not None and name_node.type == "variable_name":  # pragma: no mutate
                out.add(_text(name_node))


def _references(node: Node, name: str) -> bool:
    return any(sub.type == "variable_name" and _text(sub) == name for sub in _walk(node))


def _first_variable_name(node: Node) -> str | None:
    for sub in _walk(node):
        if sub.type == "variable_name":
            return _text(sub)
    return None  # pragma: no cover — a postfix_expression always wraps a variable_name


def _binary_operator(node: Node) -> str:
    for child in node.children:
        if not child.is_named:
            return child.type
    return ""  # pragma: no cover — a binary_expression always has an operator token # pragma: no mutate — unreachable fallback


def _let_target(text: str) -> str | None:
    match = _LET_TARGET_RE.match(text)
    return match.group(1) if match else None


# ---------------------------------------------------------------- hints


def _uses_self_location(root: Node) -> bool:
    """$0 / $BASH_SOURCE / ${BASH_SOURCE[0]} / dirname "$0" — the script cares where it lives, so a
    const rewrite running from a temp copy could change that answer."""
    return any(
        node.type == "variable_name" and _text(node) in ("0", "BASH_SOURCE") for node in _walk(root)
    )


def _uses_argv(root: Node) -> bool:
    """$1…/$@/$#/$* or getopts/shift — the script reads its own positional arguments."""
    for node in _walk(root):
        kind = node.type
        if kind == "variable_name":
            value = _text(node)
            if value.isdigit() and value != "0":
                return True
        elif kind == "special_variable_name":
            if _text(node) in ("@", "*", "#"):
                return True
        elif kind == "command":
            name_node = node.child_by_field_name("name")
            if name_node is not None and _text(name_node) in ("getopts", "shift"):
                return True
    return False
