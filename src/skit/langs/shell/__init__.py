"""Shell language support (bash/sh/zsh): the tree-sitter-bash analyzer + drift reconcile.

The tree-sitter import lives in `analyzer.py`; the registry's shell-spec builder wraps the
`from .shell import analyzer` in a try/except ImportError, so a broken grammar wheel degrades
`spec.analyzer` to None (the capability idiom) instead of crashing skit.
"""
