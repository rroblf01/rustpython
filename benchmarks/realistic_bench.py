#!/usr/bin/env python3
"""
Realistic RustPython JIT Benchmark Suite
No imports needed — works with bare RustPython.
"""
import sys

N = 1500

def fib(n):
    if n < 2:
        return n
    return fib(n-1) + fib(n-2)

def bench_fib():
    return fib(30)

def bench_nested():
    s = 0
    for i in range(100):
        for j in range(100):
            s += i * j + 1
    return s

def bench_list_comp():
    r = 0
    for x in range(50):
        lst = [y + 1 for y in range(60)]
        r += lst[0] + lst[-1]
    return r

def bench_dict():
    d = {}
    for i in range(N):
        d["k" + str(i)] = i * 2
    r = 0
    for i in range(N):
        r += d.get("k" + str(i), 0)
    return r

def bench_attr():
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

def bench_tuple():
    r = 0
    for i in range(N):
        t = (i, i+1, i+2)
        r += t[0] + t[1] + t[2]
    return r

def bench_call():
    def f(a, b, c, d):
        return a + b + c + d
    r = 0
    for i in range(N):
        r += f(i, i+1, i+2, i*2)
    return r

def bench_range():
    r = 0
    for i in range(N):
        if i % 3 == 0:
            r += i
        elif i % 5 == 0:
            r -= i
    return r

def bench_append():
    lst = []
    for i in range(N):
        lst.append(i)
        lst.append(i+1)
    return len(lst)

def bench_while():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n

def bench_contains():
    items = list(range(50))
    r = 0
    for i in range(N):
        if (i % 50) in items:
            r = r + 1
    return r

def bench_negation():
    r = 0
    for i in range(N):
        if i % 2 == 0:
            r += -i
        else:
            r += i
    return r

def bench_not():
    r = 0
    for i in range(N):
        if i % 3 != 0:
            r += 1
    return r

ALL_BENCHMARKS = [
    ("fibonacci(30)", bench_fib),
    ("nested_loops", bench_nested),
    ("list_comp", bench_list_comp),
    ("dict_ops", bench_dict),
    ("attr_access", bench_attr),
    ("tuple_build", bench_tuple),
    ("function_call", bench_call),
    ("range_iter", bench_range),
    ("list_append", bench_append),
    ("while_loop", bench_while),
    ("contains_op", bench_contains),
    ("negation", bench_negation),
    ("not_op", bench_not)]

def main():
    results = {}
    for idx in range(len(ALL_BENCHMARKS)):
        item = ALL_BENCHMARKS[idx]
        name = item[0]
        fn = item[1]
        result = fn()
        results[name] = result

    print("OK - all benchmarks completed")
    for idx2 in range(len(ALL_BENCHMARKS)):
        item = ALL_BENCHMARKS[idx2]
        name = item[0]
        print("  " + name + ": " + str(results[name]))

main()
