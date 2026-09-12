#!/usr/bin/env python3
"""向某人问候数次。"""

import argparse
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="向某人问候数次。")
    parser.add_argument("--name", default="World", help="要问候的人")
    parser.add_argument("--count", type=int, default=1, help="问候次数")
    parser.add_argument("--shout", action="store_true", help="使用大写字母")
    parser.add_argument("--names", type=Path, help="逐行问候此文件中的每个人")
    args = parser.parse_args()

    names = [args.name]
    if args.names:
        names += [line.strip() for line in args.names.read_text().splitlines() if line.strip()]

    for name in names:
        greeting = f"你好，{name}！"
        if args.shout:
            greeting = greeting.upper()
        for _ in range(args.count):
            print(greeting)


if __name__ == "__main__":
    main()
