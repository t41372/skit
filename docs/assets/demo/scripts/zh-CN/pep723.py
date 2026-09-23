#!/usr/bin/env python3
# /// script
# dependencies = []
# ///
"""报告运行环境。声明一份空的 PEP 723 依赖列表。"""

import sys


def main() -> None:
    print(f"python {sys.version_info.major}.{sys.version_info.minor}")
    print("没有第三方依赖")


if __name__ == "__main__":
    main()
