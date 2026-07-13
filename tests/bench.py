#!/usr/bin/env python3
"""
RustPython Benchmark Runner.
No imports needed — works with bare RustPython.
Usage: ./target/release/rustpython tests/bench.py
"""

import sys
import time

N = 1500

# ── Benchmarks ──────────────────────────────────────

def bench_fib():
    def fib(n):
        if n < 2:
            return n
        return fib(n-1) + fib(n-2)
    return fib(30)

def bench_nested():
    s = 0
    for i in range(100):
        for j in range(100):
            s += i * j + 1
    return s

def bench_arith():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n

def bench_list_ops():
    lst = [1, 2, 3]
    for i in range(N):
        lst.append(i)
        a = lst[0]
        b = lst[-1]
        c = len(lst)
    return (a, b, c)

def bench_dict_ops():
    d = {"a": 1, "b": 2}
    for i in range(N):
        d["c"] = i
        a = d.get("a")
        b = "a" in d
        c = len(d)
    return (a, b, c)

def bench_function_call():
    def f(x):
        return x + 1
    x = 0
    for i in range(N):
        x = f(i)
    return x

def bench_while():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n

def bench_tuple():
    r = 0
    for i in range(N):
        t = (i, i+1, i+2)
        r += t[0] + t[1] + t[2]
    return r

def bench_attr_access():
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

def bench_range():
    r = 0
    for i in range(N):
        if i % 3 == 0:
            r += i
        elif i % 5 == 0:
            r -= i
    return r

# ── Runner ──────────────────────────────────────────

ALL_BENCHMARKS = [
    ("fibonacci(30)",   bench_fib),
    ("nested_loops",    bench_nested),
    ("arithmetic",      bench_arith),
    ("list_ops",        bench_list_ops),
    ("dict_ops",        bench_dict_ops),
    ("function_call",   bench_function_call),
    ("while_loop",      bench_while),
    ("tuple_build",     bench_tuple),
    ("attr_access",     bench_attr_access),
    ("range_iter",      bench_range),
]

def main():
    print("=== RustPython Benchmark Suite ===")
    print()
    results = {}
    for name, fn in ALL_BENCHMARKS:
        # Warmup
        fn()
        # Timed run (repeated for stable measurement)
        t0 = time.time()
        n_runs = 3
        for _ in range(n_runs):
            result = fn()
        elapsed = (time.time() - t0) / n_runs
        results[name] = (result, elapsed)
        print("  %-20s %-15s %8.4f s" % (name + ":", str(result), elapsed))
    print()
    total = sum(e for _, (_, e) in results.items())
    print("  TOTAL: %.4f s" % total)
    print()
    print("OK")

main()
