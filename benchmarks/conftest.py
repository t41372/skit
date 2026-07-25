"""Pytest configuration for the CodSpeed benchmark suite.

The benchmarks live outside `tests/` on purpose: they exercise the same headless
hot paths (the language analyzers and PEP 723 parsing) but are driven by
`pytest-codspeed` under CI, and must stay clear of the repository's coverage and
mutation gates, which scope strictly to `tests/`.
"""
