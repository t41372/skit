#!/usr/bin/env python3
# /// script
# dependencies = []
# ///
"""回報執行環境。宣告一份空的 PEP 723 相依套件清單。"""

import sys


def main() -> None:
    print(f"python {sys.version_info.major}.{sys.version_info.minor}")
    print("沒有第三方相依套件")


if __name__ == "__main__":
    main()
