"""Mutation-kill pins for the fish ``argparse`` reader (``langs/fish/cli_reader.py``).

These target surviving mutants that the broad-behaviour tests in ``test_fish.py`` do not
catch: the exact ``ParamDecl`` shape a spec assembles to (binding/delivery/default/
multiple identities), the ``!validator`` split direction, the numeric-vs-plain flag flag,
and the two leading-prefix-skip edges in ``_find_argparse``. Everything is a direct call
to the real ``read_cli`` public surface — no mocking — so each assertion pins behaviour a
plan/assemble caller actually depends on. ``cli_reader`` emits no user-facing strings, so
no locale setup is needed.
"""

from __future__ import annotations

import signal
import sys
from contextlib import contextmanager

from skit.langs.fish import cli_reader as fc
from skit.params import ParamDecl


def read(src: str) -> dict[str, ParamDecl]:
    """{name: ParamDecl} for the spec strings in a fish ``argparse`` command."""
    spec = fc.read_cli(src)
    assert spec is not None
    return {f.name: f for f in spec.fields}


class _LoopTimeout(Exception):
    """Raised when a guarded call runs past its deadline (a mutant's infinite loop)."""


@contextmanager
def _deadline(seconds: float):
    """Fail (rather than hang the suite) if the body runs longer than ``seconds``.

    ``_find_argparse``'s ``j += 1`` skip advances past every leading and/or/not prefix; a
    mutant that freezes it (``j = 1``) spins forever on the second stacked prefix. Only a
    real timeout can observe that divergence, so the mutant is caught here rather than by a
    value assertion. SIGALRM interrupts the pure-Python loop between bytecodes."""
    if sys.platform == "win32":  # SIGALRM is POSIX-only; the mutant is caught on the Unix CI.
        yield
        return

    def _fire(signum: int, frame: object) -> None:  # signal-handler signature
        raise _LoopTimeout

    old = signal.signal(signal.SIGALRM, _fire)  # ty: ignore[possibly-missing-attribute]
    signal.setitimer(signal.ITIMER_REAL, seconds)  # ty: ignore[possibly-missing-attribute]
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)  # ty: ignore[possibly-missing-attribute]
        signal.signal(signal.SIGALRM, old)  # ty: ignore[possibly-missing-attribute]


# ---------------------------------------------------------------- _find_argparse prefix skip


def test_find_argparse_skips_a_lone_leading_prefix():
    # A statement that is ENTIRELY a leading conjunction (`or` on its own line) must be
    # skipped without indexing past the token list; the `j <= len(words)` mutant reads
    # words[len] and raises IndexError before the real argparse line is ever reached.
    fields = read("or\nargparse 'h/help' -- $argv\n")
    assert list(fields) == ["help"]


def test_find_argparse_advances_past_every_stacked_prefix():
    # Two stacked conjunctions (`or not argparse …`) must both be skipped so argparse is
    # still found. The `j = 1` mutant stops advancing and loops forever on the 2nd prefix.
    with _deadline(3):
        fields = read("or not argparse 'h/help' -- $argv\n")
    assert list(fields) == ["help"]


# ---------------------------------------------------------------- flag / value ParamDecl shape


def test_flag_spec_binding_and_delivery():
    # Every argparse spec is a flag-delivered, source-unanchored param. Pins the literal
    # "none"/"flag" strings against the value mutants (None / "NONE" / "XXnoneXX" /
    # "FLAG" / "XXflagXX").
    help_field = read("argparse 'h/help' -- $argv\n")["help"]
    assert help_field.binding == "none"
    assert help_field.delivery == "flag"


def test_valueless_flag_is_a_false_default_bool():
    # A no-suffix spec is a store_true bool whose default is exactly False (not None, not
    # True) — the value a form renders as an unchecked box and assemble omits when unset.
    verbose = read("argparse 'v/verbose' -- $argv\n")["verbose"]
    assert verbose.type == "bool"
    assert verbose.action == "store_true"
    assert verbose.default is False


def test_single_required_value_flag_is_not_multiple():
    # `name=` takes one required value; multiple must be exactly False (a `None` mutant
    # would slip past a plain `not field.multiple` check).
    name = read("argparse 'n/name=' -- $argv\n")["name"]
    assert name.type == "str"
    assert name.multiple is False


# ---------------------------------------------------------------- numeric-vs-plain flag


def test_single_char_short_flag_is_not_degraded():
    # A single-char short-only spec (`x`) is a normal bool flag, not the `#`-numeric
    # degraded case — the third `_parse_name` tuple element must stay False.
    x = read("argparse 'x' -- $argv\n")["x"]
    assert x.flag == "-x"
    assert x.degraded is False
    assert x.type == "bool"


def test_plain_long_flag_is_not_degraded():
    # A multi-char plain long name (`verbose`, no separator at position 1) is likewise a
    # normal flag, not degraded.
    verbose = read("argparse 'verbose' -- $argv\n")["verbose"]
    assert verbose.flag == "--verbose"
    assert verbose.degraded is False
    assert verbose.type == "bool"


# ---------------------------------------------------------------- !validator stripping


def test_validator_is_dropped_from_the_first_bang_forward():
    # `name!a!b` — the spec keeps only the text before the FIRST `!` ("name"); an rsplit
    # mutant would keep "name!a" and mis-name the field.
    fields = read("argparse 'verbose!a!b' -- $argv\n")
    assert list(fields) == ["verbose"]
    assert fields["verbose"].flag == "--verbose"
