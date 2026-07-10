#!/usr/bin/env python3
"""印出方框訊息數次——設定寫在檔案頂端。"""

MESSAGE = "來自 skit 的問候"
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
