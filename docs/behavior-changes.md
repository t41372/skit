# Deliberate behavior changes in version 0.5

Version 0.5 must be a strict superset of the behavioral oracle (`origin/main@206f9ef`, Python
`0.4.1.dev0`). This file records each place where version 0.5 does something version 0.4 does not,
so a reviewer can tell a deliberate addition from an accidental difference. `docs/parity-backlog.md`
records the opposite: a version 0.4 behavior version 0.5 does not have yet.

Add an item here when the change is user-visible. Name the product rule or the oracle line that
makes the change correct. An addition is permitted; a removal is not.

## The settings screen offers the environment-default rewrite

`skit-ui` puts a `source:normalize` control in the parameter section of a stored shell copy. It ticks
one or more constants, and the save rewrites each `NAME=value` as `NAME="${NAME:-value}"` in the
stored copy.

Version 0.4 has no such control. It offers the rewrite only as `skit params <entry> --normalize NAME`
(`src/skit/cli.py:4113-4116`), and its terminal interface points the user at that command in a hint
(`src/skit/cli.py:4014`).

Product rule 3 makes the hint the defect: "Keep the interface discoverable. A user must not need to
remember commands, keys, or script arguments." A sentence that names a command line is exactly the
case that rule refuses.

Two properties keep the control safe:

- It is strictly opt-in. The option set opens with nothing ticked, and a save that touches no tick
  carries no `source:normalize` value at all. `AGENTS.md` calls `--normalize` "the only opt-in
  semantic edit to a stored script", so a default-on box would make a rename rewrite a script.
  `every_tick_to_act_offer_opens_empty_and_a_save_that_touches_none_carries_none` pins this.
- It never offers a constant that already reads `${NAME:-value}`. That form is the result of this
  rewrite, and the normalizer refuses a value that names itself
  (`crates/skit-language/src/semantic/shell.rs:747-751`), so offering it again would produce only a
  refusal.

## The resync is a control, not only a chord

Version 0.4 binds `Ctrl+R` to `action_resync`, which reads the script's parameter definitions again
and rebuilds the screen (`src/skit/tui_settings.py:269`, `:908-926`). There is no click target and no
way to take the request back.

Version 0.5 gives the same chord, and the chord reaches a visible checkbox in the parameter section.
Product rule 2 requires it: "Keep each TUI action available by keyboard and mouse." The checkbox is
also reversible, so a person who presses the chord by mistake can untick it.

The request now applies when the save runs, rather than immediately. The pre-save report version 0.4
prints is not built yet; it is recorded in `docs/parity-backlog.md`.

## The preset deep link moves the keyboard, not only the viewport

Version 0.4's Library gives `s` to `action_settings(section="presets")` (`src/skit/tui.py:991-992`),
and the settings screen scrolls its body to that section on mount
(`src/skit/tui_settings.py:876-882`). The keyboard stays where the screen put it, which is the name
box, so the first key press after `s` types into the name.

Version 0.5 also puts the keyboard on the first preset. Product rule 2 asks every action to have a
keyboard path, and a person who pressed `s` came to act on a preset: `Space` deletes the first one
immediately, rather than typing a space into an unrelated field.

A section with nothing to edit keeps the anchor instead, so an entry with no presets still lands on
the sentence that says where presets come from. The anchor is released by the first keyboard move,
after which the viewport follows the focus as it does on every other screen.

## The add-a-parameter box takes more than one name

Version 0.4 reads one name from its add box and makes one declaration
(`src/skit/tui_settings.py:719`, `:747-749`). Version 0.5 splits the same box on commas and spaces,
so one save can add several parameters. The single-name case behaves the same way.
