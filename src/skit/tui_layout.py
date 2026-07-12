"""Shared responsive-layout policy: one set of size tiers for every skit screen.

Textual turns these breakpoints into CSS classes on whichever screen is active —
declared once per App, they reach every screen in the stack, modals included, at
mount and on every live resize. All size-aware styling keys off the tier classes
declaratively; no screen does its own width/height math in Python. Exactly one
class per axis is set at a time (Textual tiers are exclusive), so a rule that
covers "short or worse" must name both `-h-short` and `-h-tiny`.

Width tiers:
- ``-w-narrow`` (< 80 cols): the Library gives the whole row to the list (spec §1),
  horizontal RadioSets stack vertically, the run form's preset row stacks.
- ``-w-normal`` (>= 80 cols): the full side-by-side layout.

Height tiers:
- ``-h-tall`` (>= 28 rows): vertical room to spare — the footer key area wraps
  without a cap, so every chip is visible outright even on a sliver-narrow window.
  Narrow AND tall is the portrait shape: the Library stacks the detail pane below
  the list instead of hiding it.
- ``-h-normal`` (16-27 rows): everything; the footer key area shows up to three
  wrapped lines per bar.
- ``-h-short`` (10-15 rows): chrome slims down — the Library's search box flattens
  to a single borderless row and the footer key area shows one line per bar.
- ``-h-tiny`` (< 10 rows): the degradation floor — one visible footer line total in
  the Library; the status line stays (it is the error/feedback channel).

The footer caps trim VISIBILITY, never capability: key rows live in a scrollable
KeysBar (tui_footer), so chips past the cap stay wheel-reachable for the mouse and
every key keeps firing for the keyboard. Chips are ordered most important first, so
the visible lines are always the most useful ones. Modals never exceed the screen;
below the tiny tier skit clips predictably rather than reflowing further.
"""

from __future__ import annotations

NARROW_WIDTH = 80
TALL_HEIGHT = 28
SHORT_HEIGHT = 16
TINY_HEIGHT = 10

HORIZONTAL_BREAKPOINTS: list[tuple[int, str]] = [
    (0, "-w-narrow"),
    (NARROW_WIDTH, "-w-normal"),
]
VERTICAL_BREAKPOINTS: list[tuple[int, str]] = [
    (0, "-h-tiny"),
    (TINY_HEIGHT, "-h-short"),
    (SHORT_HEIGHT, "-h-normal"),
    (TALL_HEIGHT, "-h-tall"),
]
