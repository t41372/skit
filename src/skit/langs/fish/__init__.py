"""fish shell support: a hand scanner (no tree-sitter — there is no maintained PyPI grammar
wheel, and fish syntax is regular enough to scan directly: no heredocs, simple quoting).

**v1 scope (deliberately narrow — documented in `analyzer.py`):**

- `analyze()` emits ONLY env-default candidates: the fish idiom ``set -q NAME; or set NAME
  value`` (env delivery, zero rewrite — fish sees inherited environment variables as ordinary
  variables, so an env overlay works natively). const/read detection is DEFERRED: their
  delivery would need an injector that does not exist for fish yet, and emitting a candidate
  skit cannot deliver would be dishonest. The env idiom needs no injector, so it is safe to
  surface and manage today.
- `cli_reader.read_cli()` reads the builtin ``argparse 'h/help' 'n/name=' … -- $argv`` spec
  strings into flag-delivery params — fully registered.

There is no injector and no import guard: the scanner is pure stdlib, so nothing can fail to
import. const/read injection is a later increment.
"""
