# dict/list 常量不是候選(非純 literal 標量),必須原樣保留
CONFIG = {"retries": 3}
NAME = "job"
print(CONFIG["retries"], NAME)
