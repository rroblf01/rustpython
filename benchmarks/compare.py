#!/usr/bin/env python3
"""
Minimal benchmark runner.
Works on both CPython and RustPython (try/except for time module).
Run:
  python3 benchmarks/compare.py          # CPython with timings
  target/release/rustpython benchmarks/compare.py   # RustPython
"""

N = 50000

def bench(name, fn):
    has_time = False
    try:
        import time as _t
        _t.perf_counter
        has_time = True
    except Exception:
        has_time = False
    fn()
    if has_time:
        t0 = _t.perf_counter()
        for _ in range(3):
            fn()
        t = (_t.perf_counter() - t0) / 3
        print(name, "  ", round(t * 1000, 2), "ms", str(fn()))
    else:
        print(name, "  ", str(fn()))

# Arithmetic
def b_arith():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n
bench("arithmetic", b_arith)

# Global lookup
def b_global():
    x = 0
    for i in range(N):
        x = N
    return x
bench("global_lookup", b_global)

# List ops
def b_list():
    lst = [1, 2, 3]
    for i in range(N):
        lst.append(i)
        a = lst[0]
        b = lst[-1]
        c = len(lst)
    return (a, b, c)
bench("list_ops", b_list)

# Dict ops
def b_dict():
    d = {"a": 1, "b": 2}
    for i in range(N):
        d["c"] = i
        a = d.get("a")
        b = "a" in d
        c = len(d)
    return (a, b, c)
bench("dict_ops", b_dict)

# Function call
def b_call():
    def f(x):
        return x + 1
    x = 0
    for i in range(N):
        x = f(i)
    return x
bench("function_call", b_call)

# While loop
def b_while():
    i = 0
    n = 0
    while i < N:
        n += i
        i += 1
    return n
bench("while_loop", b_while)
