"""JavaScript & TypeScript support: one package serving both `js` and `ts` kinds.

The TypeScript grammar is a superset of JavaScript, so a single analyzer / parseArgs reader /
injector handles both — the kind selects the grammar through a `lang` argument ("js" →
tree-sitter-javascript, "ts" → tree-sitter-typescript, "tsx" → the TSX dialect).

The tree-sitter imports live in `analyzer.py` (re-used by `cli_reader.py` and `inject.py`); the
registry's js/ts-spec builders wrap `from .javascript import analyzer, cli_reader, inject` in a
try/except ImportError, so a broken grammar wheel degrades those capabilities to None (the capability
idiom) instead of crashing skit. `io.py` (the `// ///` block engine) imports no grammar and always
works, so declared-parameter management survives even a degraded grammar.
"""
