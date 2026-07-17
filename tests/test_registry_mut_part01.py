"""Mutation-kill tests for the two zero-capability registry builders.

`registry._exe_spec` and `registry._command_spec` bake fixed data into their LangSpecs
(kind key, family, badge glyph, launch strategy, and — for command — the takes_argv=False
affordance). Nothing else asserts these exact values, so each field is pinned here through
the real `registry.spec_for` resolution path. Field values are the English source of truth.
"""

from __future__ import annotations

from skit.langs import launch, registry


def test_exe_spec_fields_and_launch_strategy():
    spec = registry.spec_for("exe")
    assert spec is not None
    assert spec.kind == "exe"
    assert spec.family == "binary"
    assert spec.glyph == "▶"  # ▶ — the exe badge glyph
    # exe runs its target directly (no interpreter, no shell wrapper).
    assert isinstance(spec.launch, launch.DirectLaunch)


def test_command_spec_fields_and_launch_strategy():
    spec = registry.spec_for("command")
    assert spec is not None
    assert spec.kind == "command"
    assert spec.family == "template"
    assert spec.glyph == "$"  # the command-template badge glyph
    # a command is a shell template expanded through TemplateLaunch.
    assert isinstance(spec.launch, launch.TemplateLaunch)


def test_command_spec_does_not_take_argv():
    # takes_argv=False overrides the True default: a command's "arguments" are its
    # placeholders, so run's reuse-last-args affordance must not append a remembered
    # argv tail. `is False` also pins it against a None mutation.
    spec = registry.spec_for("command")
    assert spec is not None
    assert spec.takes_argv is False
