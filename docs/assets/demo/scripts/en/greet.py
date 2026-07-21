#!/usr/bin/env python3
"""Greet someone a number of times."""

import argparse
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Greet someone a number of times.")
    parser.add_argument("--name", default="World", help="Who to greet")
    parser.add_argument("--count", type=int, default=1, help="How many times to greet")
    parser.add_argument("--shout", action="store_true", help="Greet in UPPERCASE")
    parser.add_argument("--names", type=Path, help="Also greet everyone in this file, one per line")
    args = parser.parse_args()

    names = [args.name]
    if args.names:
        names += [line.strip() for line in args.names.read_text().splitlines() if line.strip()]

    for name in names:
        greeting = f"Hello, {name}!"
        if args.shout:
            greeting = greeting.upper()
        for _ in range(args.count):
            print(greeting)


if __name__ == "__main__":
    main()
