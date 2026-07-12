#!/usr/bin/env python3
import sys

N = 2000

def get_time():
    try:
        import time
        return time.perf_counter()
    except ImportError:
        return 0

def bench(name, fn):
    t0 = get_time()
    result = fn()
    t = get_time() - t0
    if t > 0:
        print(name + "\t" + str(round(t, 4)) + "\t" + str(result))
    else:
        print(name + "\t?\t" + str(result))

def fib(n):
    if n < 2:
        return n
    return fib(n-1) + fib(n-2)

def b_fib():
    return fib(28)

def b_nested():
    s = 0
    for i in range(100):
        for j in range(100):
            s += i * j + 1
    return s

def b_list_comp():
    r = 0
    for x in range(50):
        lst = [y + 1 for y in range(60)]
        r += lst[0] + lst[-1]
    return r

def b_dict():
    d = {}
    for i in range(N):
        d["k" + str(i)] = i * 2
    r = 0
    for i in range(N):
        r += d.get("k" + str(i), 0)
    return r

def b_attr():
    class Vec:
        def __init__(self, x, y):
            self.x = x
            self.y = y
        def dot(self, other):
            return self.x * other.x + self.y * other.y
    v1 = Vec(3, 4)
    v2 = Vec(5, 6)
    r = 0
    for i in range(N):
        r += v1.dot(v2)
    return r

def b_tuple():
    r = 0
    for i in range(N):
        t = (i, i+1, i+2)
        r += t[0] + t[1] + t[2]
    return r

def b_call():
    def f(a, b, c, d):
        return a + b + c + d
    r = 0
    for i in range(N):
        r += f(i, i+1, i+2, i*2)
    return r

def b_arith():
    r = 0
    for i in range(N):
        r += i * 2
        r -= i // 3
        if i % 2 == 0:
            r += -i
        else:
            r += i
    return r

def b_append():
    lst = []
    for i in range(N):
        lst.append(i)
        lst.append(i+1)
    return len(lst)

def b_while():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n

def b_contains():
    items = list(range(50))
    r = 0
    for i in range(N):
        if (i % 50) in items:
            r = r + 1
    return r

ALL = [
    ("fibonacci", b_fib),
    ("nested", b_nested),
    ("list_comp", b_list_comp),
    ("dict", b_dict),
    ("attr", b_attr),
    ("tuple", b_tuple),
    ("call", b_call),
    ("arith", b_arith),
    ("append", b_append),
    ("while", b_while),
    ("contains", b_contains)]

is_rust = "--rust" in sys.argv

if not is_rust:
    import time

print("# Benchmark results")
print("# N = " + str(N))
if not is_rust:
    print("# Mode: CPython (with timings)")
    print("# name\ttime(s)\tresult")
    for idx in range(len(ALL)):
        item = ALL[idx]
        name = item[0]
        fn = item[1]
        bench(name, fn)
else:
    print("# Mode: RustPython (no timings)")
    for idx in range(len(ALL)):
        item = ALL[idx]
        name = item[0]
        fn = item[1]
        result = fn()
        print(name + "\t<time>\t" + str(result))
