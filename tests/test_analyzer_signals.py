"""Add-time signals (UX spec §0): accumulator demotion, argv detection, filename hints."""

from __future__ import annotations

from skit import analyzer

IMAGE_STITCH = """
from PIL import Image
import sys

images = [Image.open(x) for x in sys.argv[1:]]

y_offset = 0
for im in images:
    im.paste(im, (0, y_offset))
    y_offset += im.size[1]

im.save('output_long_image.jpg')
print("done")
"""


def test_accumulator_is_demoted():
    result = analyzer.analyze(IMAGE_STITCH)
    y = next(c for c in result.candidates if c.name == "y_offset")
    assert y.demoted is True
    assert y.demotion == "accumulator"


def test_clean_constant_is_not_demoted():
    result = analyzer.analyze("OUTPUT = 'out.jpg'\nprint(OUTPUT)\n")
    out = next(c for c in result.candidates if c.name == "OUTPUT")
    assert out.demoted is False
    assert out.demotion == ""


def test_reassignment_inside_while_loop_demotes():
    result = analyzer.analyze("count = 0\nwhile go():\n    count = count + 1\n")
    c = next(c for c in result.candidates if c.name == "count")
    assert c.demoted is True


def test_augassign_outside_loop_still_demotes():
    result = analyzer.analyze("total = 0\ntotal += cost()\n")
    c = next(c for c in result.candidates if c.name == "total")
    assert c.demoted is True


def test_uses_argv_detected():
    assert analyzer.analyze(IMAGE_STITCH).uses_argv is True
    assert analyzer.analyze("print('no args')\n").uses_argv is False
    assert analyzer.analyze("import sys\nn = len(sys.argv)\n").uses_argv is True


def test_filename_literal_hint_found():
    assert analyzer.analyze(IMAGE_STITCH).filename_literals == ["output_long_image.jpg"]


def test_no_hint_for_named_constant_usage():
    # Once the literal is extracted to a named constant, the call site holds a Name,
    # not a Constant — the hint disappears (the edit→rescan loop from the simulation).
    text = "OUTPUT = 'output_long_image.jpg'\nsave(OUTPUT)\n"
    assert analyzer.analyze(text).filename_literals == []


def test_hint_excludes_non_filenames():
    text = (
        "new('RGB')\n"  # no extension
        "log('finished: output.jpg now ready')\n"  # sentence, has spaces
        "get('https://example.com/a.zip')\n"  # URL
        "ver('3.14')\n"  # numeric "extension" is a version
    )
    assert analyzer.analyze(text).filename_literals == []


def test_hint_dedupes_and_caps_at_three():
    text = "f('a.png')\nf('a.png')\nf('b.png')\nf('c.png')\nf('d.png')\n"
    assert analyzer.analyze(text).filename_literals == ["a.png", "b.png", "c.png"]
