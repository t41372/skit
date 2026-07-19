"""The prompt kind's language package (docs/design/prompt.md).

Pure stdlib, no grammar, no import guard: placeholder detection is a regex scan
(analyzer.py) and value delivery is raw text substitution (render.py). Everything here
runs on the launch path, so the stdlib-only rule for launch code holds by construction.
"""
