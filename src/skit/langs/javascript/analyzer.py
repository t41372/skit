"""JS/TS analyzer: const-parameter detection for JavaScript & TypeScript via tree-sitter.

One analyzer serves BOTH kinds — the TypeScript grammar is a superset of JavaScript, so the only
difference is which `tree_sitter.Language` a `lang` selects (`js` → tree-sitter-javascript, `ts` →
tree-sitter-typescript, `tsx` → the TSX dialect). Detection is a **node walk** over the parse tree
(no query strings — the mutmut gate needs real code to mutate, not opaque S-expression literals),
parsing `text.encode("utf-8")` so tree-sitter's absolute byte offsets line up with the source.

Detection (docs/design/multilang.md, §"JS/TS"):

- **const**: a top-level `lexical_declaration` whose `kind` is `const` (a direct child of `program`),
  with a `variable_declarator` whose name is a plain `identifier` (array/object destructuring is
  skipped) and whose value is a *literal* — a `number`, a `string` (text carried in a
  `string_fragment` child), or `true`/`false`. A `template_string` is EXCLUDED (it may interpolate),
  as are object/array literals and any other expression. Last-write-wins dedupe by name (first slot
  kept, last value wins), leading-underscore names skipped — exactly like Python and shell.
- **accumulator demotion**: a `let`/`var` top-level declaration with a literal value is still offered
  as a candidate, but DEMOTED "accumulator" (a reassignable binding is a working variable, not a
  parameter — the user may opt in, unchecked by default). A `const` that is nonetheless reassigned
  (`X = …`, `X += …`, `X++` at any depth) is likewise demoted.
- **types**: `int` iff the number text matches `^-?\\d+$`, `float` otherwise (a `number` node is
  always numeric, never a string — an exotic literal python's float() can't parse, `0xFF`/`1e3`/
  `100n`, keeps its source text as the informational default); `str` for a string; `bool` for
  `true`/`false`. Unlike shell, JS DOES have booleans, so `true`/`false` infer `bool`.
- **secret**: `params.is_secret_name(name)` — the same universal heuristic every language runs.
- **degradation honesty**: any `tree.root_node.has_error` ⇒ `Analysis(syntax_error=True)` (empty),
  so a construct a grammar can't parse degrades honestly to Tier-0 rather than guessing.

JS candidates are const-only (there is no env-default idiom and no interactive-read equivalent in
scope), so `binding` is always `"const"` and delivery is always inject.
"""

from __future__ import annotations

import re
from typing import TYPE_CHECKING

import tree_sitter_javascript
import tree_sitter_typescript
from tree_sitter import Language, Parser

from ... import analysis
from ...analysis import Analysis, Candidate
from ...params import is_secret_name

if TYPE_CHECKING:
    from collections.abc import Iterator

    from tree_sitter import Node

    from ...analysis import Report
    from ...params import ParamDecl

_JS = Language(tree_sitter_javascript.language())
_TS = Language(tree_sitter_typescript.language_typescript())
_TSX = Language(tree_sitter_typescript.language_tsx())

# The grammar each `lang` selects. `.get(..., _JS)` keeps the resolver total (an unknown lang falls
# back to plain JavaScript rather than raising) while the three real values pick their own dialect.
_LANGUAGES = {"js": _JS, "ts": _TS, "tsx": _TSX}

# The declaration keywords that produce a reassignable binding (a working variable, not a constant):
# a `let`/`var` candidate is demoted to "accumulator" on sight, like an augmented-assigned Python name.
_REASSIGNABLE = frozenset({"let", "var"})

_INT_RE = re.compile(r"-?\d+")
_FLOAT_RE = re.compile(r"-?\d+\.\d+")


def language_for(lang: str) -> Language:
    """The tree-sitter `Language` for a `lang` selector ("js" / "ts" / "tsx"); JavaScript for
    anything else. Shared by the analyzer, the parseArgs reader, and the injector so all three parse
    a given entry under exactly the same grammar."""
    return _LANGUAGES.get(lang, _JS)


def analyze(text: str, *, lang: str = "js") -> Analysis:  # pragma: no mutate — default-lang mutants equivalent: language_for() maps js/JS/XXjsXX all to _JS  # fmt: skip
    """Detect candidate parameters in a JS/TS script. On any parse error, return an empty result
    (no exception; add can still take the script into the store — honest Tier-0 degradation)."""
    root = Parser(language_for(lang)).parse(text.encode("utf-8")).root_node  # pragma: no mutate — "utf-8"/"UTF-8" name the same codec (case-insensitive)  # fmt: skip
    if root.has_error:
        return Analysis(syntax_error=True)
    consts = _const_candidates(root)
    mutated = _mutated_names(root)
    for c in consts:
        if c.name in mutated:
            c.demoted = True
            c.demotion = "accumulator"
    return Analysis(candidates=consts)


def reconcile(text: str, specs: list[ParamDecl], *, lang: str = "js") -> Report:  # pragma: no mutate — default-lang mutants equivalent: lang only feeds language_for(), which maps js/JS/XXjsXX all to _JS  # fmt: skip
    """Reconcile the [tool.skit] definitions with the script's current content (wires the JS/TS
    analyzer into the neutral reconcile — see skit.analysis.reconcile)."""
    return analysis.reconcile(text, specs, analyze=lambda t: analyze(t, lang=lang))


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
        return ""  # pragma: no mutate — unreachable defensive branch (node.text is never None)
    return node.text.decode("utf-8")  # pragma: no mutate — "utf-8"/"UTF-8" name the same codec


# ---------------------------------------------------------------- literals & types


def _literal_value(node: Node) -> tuple[str, str | int | float | bool] | None:
    """(type, typed-default) for a value node when it is a plain literal, else None.

    The literal domain is exactly {number, string, true, false} — a `template_string` (possible
    interpolation), an object/array literal, or any other expression returns None and is not offered
    as a parameter. This is the ONE predicate the analyzer and the injector share, so what the form
    offers is exactly what a run can safely rewrite."""
    kind = node.type
    if kind == "number":
        return _infer_number(_text(node))
    if kind == "string":
        return "str", _string_value(node)
    if kind == "true":
        return "bool", True
    if kind == "false":
        return "bool", False
    return None


def _infer_number(text: str) -> tuple[str, str | int | float]:
    """(type, value) for a `number` node's text. int by `^-?\\d+$`, else float — a number node is
    numeric by construction, never a string. A literal python's float() can't parse (hex `0xFF`,
    exponent-less-dot forms, BigInt `100n`) keeps its source text as the informational default; the
    value the user actually injects is coerced separately, so nothing here needs to round-trip it."""
    if _INT_RE.fullmatch(text):
        return "int", int(text)
    if _FLOAT_RE.fullmatch(text):
        return "float", float(text)
    return "float", text


def _string_value(node: Node) -> str:
    """The inner text of a string literal: the concatenation of its named children (the
    `string_fragment` and `escape_sequence` runs — the surrounding quotes are anonymous children, so
    they're excluded). An empty string has no named children, so it yields "" — exactly right."""
    return "".join(_text(child) for child in node.named_children)


# ---------------------------------------------------------------- const


def _toplevel_declarations(root: Node) -> Iterator[tuple[Node, str]]:
    """Yield (variable_declarator, keyword) for each top-level declaration binding: the
    `variable_declarator` children of a `lexical_declaration` (kind const/let) or a
    `variable_declaration` (always `var`). A declaration can bind several names in one statement
    (`const A = 1, B = 2`), so every declarator is yielded, each tagged with its keyword."""
    for child in root.named_children:
        if child.type == "lexical_declaration":
            keyword_node = child.child_by_field_name("kind")
            if keyword_node is None:  # pragma: no cover — a lexical_declaration always carries kind
                continue  # pragma: no mutate — unreachable (kind always present)
            keyword = keyword_node.type
        elif child.type == "variable_declaration":
            keyword = "var"
        else:
            continue
        for sub in child.named_children:
            if sub.type == "variable_declarator":
                yield sub, keyword


def _const_candidates(root: Node) -> list[Candidate]:
    """Top-level literal declarations, last-write-wins deduped by name (first slot kept, last value
    wins — like Python/shell, so the injected value matches what the name holds once the module
    finishes evaluating). `let`/`var` bindings are included but demoted to "accumulator" on sight."""
    out: list[Candidate] = []
    index_by_name: dict[str, int] = {}
    for declarator, keyword in _toplevel_declarations(root):
        name_node = declarator.child_by_field_name("name")
        value_node = declarator.child_by_field_name("value")
        if name_node is None or name_node.type != "identifier" or value_node is None:
            continue  # a destructuring pattern (object/array) or a bare `let x;` with no value
        name = _text(name_node)
        if name.startswith("_"):
            continue  # conventionally private/internal values; not treated as parameters
        literal = _literal_value(value_node)
        if literal is None:
            continue  # a template string, object/array, call, or any other non-literal expression
        type_name, default = literal
        candidate = Candidate(
            binding="const",
            name=name,
            type=type_name,
            default=default,
            lineno=declarator.start_point[0] + 1,
            secret=is_secret_name(name),
        )
        if keyword in _REASSIGNABLE:
            candidate.demoted = True
            candidate.demotion = "accumulator"
        if name in index_by_name:
            out[index_by_name[name]] = candidate  # last occurrence's data wins; keep first slot
        else:
            index_by_name[name] = len(out)
            out.append(candidate)
    return out


# ---------------------------------------------------------------- demotions


def _mutated_names(root: Node) -> set[str]:
    """Names reassigned anywhere in the file: a plain `X = …` (`assignment_expression`), a compound
    `X += …` (`augmented_assignment_expression`), or an increment/decrement `X++`/`--X`
    (`update_expression`). A const that is nonetheless reassigned is a working variable, so it is
    demoted like Python's augmented-assigned constants."""
    out: set[str] = set()
    for node in _walk(root):
        kind = node.type
        if kind in ("assignment_expression", "augmented_assignment_expression"):
            _collect_named_target(node.child_by_field_name("left"), out)
        elif kind == "update_expression":
            _collect_named_target(node.child_by_field_name("argument"), out)
    return out


def _collect_named_target(target: Node | None, out: set[str]) -> None:
    """Record `target`'s name when it is a plain identifier (a `member_expression` or subscript
    target like `obj.x = …` reassigns a property, not the top-level binding, so it is ignored)."""
    if target is not None and target.type == "identifier":  # pragma: no mutate — and->or equivalent: only adds non-identifier target texts that never match a bare const name, and a None target is unreachable in an error-free tree  # fmt: skip
        out.add(_text(target))


# ---------------------------------------------------------------- external imports

# Module names node ships built in (the `node:`-prefixable set) — importing one of these bare
# ("fs", "path") is NOT a package dependency. Source: `node -p "require('module').builtinModules"`
# (node 22 LTS), minus internals. deno/bun implement the same set for compatibility.
_NODE_BUILTINS = frozenset(
    {
        "assert",
        "async_hooks",
        "buffer",
        "child_process",
        "cluster",
        "console",
        "constants",
        "crypto",
        "dgram",
        "diagnostics_channel",
        "dns",
        "domain",
        "events",
        "fs",
        "http",
        "http2",
        "https",
        "inspector",
        "module",
        "net",
        "os",
        "path",
        "perf_hooks",
        "process",
        "punycode",
        "querystring",
        "readline",
        "repl",
        "stream",
        "string_decoder",
        "sys",
        "timers",
        "tls",
        "trace_events",
        "tty",
        "url",
        "util",
        "v8",
        "vm",
        "wasi",
        "worker_threads",
        "zlib",
    }
)

# Specifier schemes that never name an npm package: node builtins, deno's own registries, URLs,
# and inline data. (npm:chalk IS a package, but one deno resolves natively without node_modules —
# declaring it would double-manage it, so it is skipped too.)
_NON_PACKAGE_PREFIXES = ("node:", "npm:", "jsr:", "http:", "https:", "data:", "file:", "bun:")


def external_imports(text: str, *, lang: str = "js") -> list[str]:  # pragma: no mutate — default-lang mutants equivalent: language_for() maps js/JS/XXjsXX all to _JS  # fmt: skip
    """The bare package names this script imports — its npm dependencies as the source reveals
    them, in first-appearance order. Covers static `import`/`export … from`, dynamic `import()`,
    and CJS `require()`. Relative/absolute paths, `node:` builtins (bare or prefixed), and
    URL/scheme specifiers are excluded; a deep import ("lodash/fp", "@scope/pkg/sub") maps to its
    package root. A file that doesn't parse yields [] — the same honest degradation as analyze()."""
    root = Parser(language_for(lang)).parse(text.encode("utf-8")).root_node  # pragma: no mutate — "utf-8"/"UTF-8" name the same codec (case-insensitive)  # fmt: skip
    if root.has_error:
        return []
    out: list[str] = []
    for node in _walk(root):
        source = _import_source(node)
        if source is None:
            continue
        package = _package_name(source)
        if package is not None and package not in out:
            out.append(package)
    return out


def _import_source(node: Node) -> str | None:
    """The string literal a node imports from, or None when the node imports nothing: the `source`
    field of an `import_statement`/`export_statement`, or the single string argument of a
    `require(…)` / dynamic `import(…)` call. A non-literal specifier (`require(name)`) is None —
    skit only reports what it can read statically."""
    if node.type in ("import_statement", "export_statement"):
        source = node.child_by_field_name("source")
        return _string_value(source) if source is not None and source.type == "string" else None
    if node.type != "call_expression":
        return None
    callee = node.child_by_field_name("function")
    if callee is None or _text(callee) not in ("require", "import"):
        return None
    arguments = node.child_by_field_name("arguments")
    if arguments is None:  # pragma: no cover — a parsed call_expression always carries arguments
        return None
    strings = [a for a in arguments.named_children if a.type == "string"]
    if len(strings) != 1 or len(arguments.named_children) != 1:
        return None  # not the plain require("pkg") shape; report nothing rather than guess
    return _string_value(strings[0])


def _package_name(source: str) -> str | None:
    """The npm package a specifier names, or None when it isn't one (relative/absolute path,
    builtin, URL scheme, or Node subpath import). "@scope/pkg/deep" → "@scope/pkg";
    "lodash/fp" → "lodash". A "#"-prefixed specifier is a Node subpath import (the package.json
    "imports" field) — a private internal mapping, never an installable package."""
    if not source or source.startswith((".", "/", "#")) or source.startswith(_NON_PACKAGE_PREFIXES):
        return None
    parts = source.split("/")
    package = "/".join(parts[:2]) if source.startswith("@") else parts[0]
    # A scoped specifier must be "@scope/name" with both halves present; a bare "@scope", an
    # empty scope ("@/pkg"), or an empty name ("@scope/") names no package.
    scoped_malformed = source.startswith("@") and (
        len(parts) < 2 or len(parts[0]) < 2 or not parts[1]
    )
    if package in _NODE_BUILTINS or scoped_malformed:
        return None
    return package
