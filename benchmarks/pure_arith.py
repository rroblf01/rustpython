n = 0
for i in range(50000):
    n += i
    n -= i // 2
    n *= 2
    n //= 3
    n %= 1000
print(n)
