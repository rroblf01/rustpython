#!/usr/bin/env python3
"""Benchmark comparison runner — outputs timing table.
Usage:
    python3 benchmarks/compare_bench.py          # CPython (with timings)
    target/release/rustpython benchmarks/compare_bench.py   # RustPython (returns 0, no timings inside)
"""

import sys

N = 50000

BENCHES = []

def bench(name, fn):
    BENCHES.append((name, fn))

# ── Arithmetic ──
@bench.register
def arithmetic():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n

# ── Global lookup ──
@bench.register
def global_lookup():
    x = 0
    for i in range(N):
        x = N
        y = True
    return x

# ── List ops ──
@bench.register
def list_ops():
    lst = [1, 2, 3]
    for i in range(N):
        lst.append(i)
        a = lst[0]
        b = lst[-1]
        c = len(lst)
    return a

# ── Dict ops ──
@bench.register
def dict_ops():
    d = {"a": 1, "b": 2}
    for i in range(N):
        d["c"] = i
        a = d.get("a")
        b = "a" in d
        c = len(d)
    return a

# ── Function call ──
@bench.register
def function_call():
    def f(x):
        return x + 1
    x = 0
    for i in range(N):
        x = f(i)
    return x

# ── While loop ──
@bench.register
def while_loop():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n

# ── Fibonacci (recursive) ──
@bench.register
def fibonacci():
    def fib(n):
        if n < 2:
            return n
        return fib(n-1) + fib(n-2)
    return fib(28)
