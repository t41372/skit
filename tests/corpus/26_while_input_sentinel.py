total = 0
while True:
    line = input("num or q: ")
    if line == "q":
        break
    total += int(line)
print(total)
