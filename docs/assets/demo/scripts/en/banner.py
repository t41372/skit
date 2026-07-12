#!/usr/bin/env python3
"""Print a boxed message a few times — settings live at the top."""

MESSAGE = "Hello from skit"
TIMES = 3
WIDTH = 40


def main() -> None:
    bar = "=" * WIDTH
    for _ in range(TIMES):
        print(bar)
        print(MESSAGE.center(WIDTH))
    print(bar)


if __name__ == "__main__":
    main()
