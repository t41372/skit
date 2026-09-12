#!/usr/bin/env python3
# /// script
# dependencies = []
# ///
"""Report the run environment. Declares an empty PEP 723 dependency list."""

import sys


def main() -> None:
    print(f"python {sys.version_info.major}.{sys.version_info.minor}")
    print("no third-party dependencies")


if __name__ == "__main__":
    main()
