MODE = "fast"


def switch():
    global MODE
    MODE = MODE.upper()


switch()
print(MODE)
