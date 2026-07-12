print("hello from rustpython")
n = 0
for i in range(10000):
    n += i
    n -= -i
    if i in (1, 2, 3):
        n += 1
print("n =", n)
print("done")
