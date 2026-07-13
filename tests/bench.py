# RustPython Benchmark Suite
# No imports needed

N = 5000

def bench_arith():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n

def bench_while():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n

def bench_call():
    def f(x):
        return x + 1
    x = 0
    for i in range(N):
        x = f(i)
    return x

def bench_list():
    lst = []
    for i in range(N):
        lst.append(i)
    return len(lst)

print("=== RustPython Benchmark ===")
r1 = bench_arith()
r2 = bench_while()
r3 = bench_call()
r4 = bench_list()
print("  arithmetic:", r1)
print("  while_loop:", r2)
print("  func_call:", r3)
print("  list_append:", r4)
print("OK")
