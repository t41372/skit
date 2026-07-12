#!/usr/bin/env python3
"""Greet someone a number of times."""

import argparse


def main() -> None:
    parser = argparse.ArgumentParser(description="Greet someone a number of times.")
    parser.add_argument("--name", default="World", help="Who to greet")
    parser.add_argument("--count", type=int, default=1, help="How many times to greet")
    parser.add_argument("--shout", action="store_true", help="Greet in UPPERCASE")
    args = parser.parse_args()

    greeting = f"Hello, {args.name}!"
    if args.shout:
        greeting = greeting.upper()
    for _ in range(args.count):
        print(greeting)


if __name__ == "__main__":
    main()
