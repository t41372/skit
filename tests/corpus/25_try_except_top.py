PATH = "data.txt"
try:
    open(PATH).close()
except OSError:
    print("missing", PATH)
