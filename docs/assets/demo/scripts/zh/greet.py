#!/usr/bin/env python3
"""向某人打招呼數次。"""

import argparse
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="向某人打招呼數次。")
    parser.add_argument("--name", default="World", help="要向誰打招呼")
    parser.add_argument("--count", type=int, default=1, help="打招呼幾次")
    parser.add_argument("--shout", action="store_true", help="用大寫大喊")
    parser.add_argument("--names", type=Path, help="也向這個檔案裡的每個人打招呼，一行一個")
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
