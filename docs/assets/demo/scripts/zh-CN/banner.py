#!/usr/bin/env python3
"""多次打印方框消息。设置位于文件顶部。"""

MESSAGE = "来自 skit 的问候"
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
