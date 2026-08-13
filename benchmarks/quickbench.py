#!/usr/bin/env python3
"""Quick microbenchmark suite — runs under BOTH RustPython and CPython.

Usage:
    target/release/rustpython benchmarks/quickbench.py
    python3 benchmarks/quickbench.py

Prints one line per benchmark: `name<TAB>seconds<TAB>result`.
`benchmarks/bench_trend.py` parses this output to compare the two.
"""

import time

N = 40000


def bench(name, fn):
    t0 = time.perf_counter()
    r = fn()
    t = time.perf_counter() - t0
    print("%s\t%.6f\t%s" % (name, t, r))


def b_arith():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n


def b_list():
    lst = [1, 2, 3]
    for i in range(N):
        lst.append(i)
        a = lst[0]
        b = lst[-1]
        c = len(lst)
    return len(lst)


def b_str_concat():
    # Repeated `+` (CPython has an amortized-growth fast path; a quadratic
    # implementation here is a big, easy-to-measure red flag).
    s = ""
    for i in range(N):
        s = s + "x"
    return len(s)


def b_dict():
    d = {}
    for i in range(N):
        d[i] = i
    return len(d)


def b_call():
    def f(x):
        return x + 1
    s = 0
    for i in range(N):
        s = f(s)
    return s


def b_while():
    i = 0
    s = 0
    while i < N:
        s += i
        i += 1
    return s


def b_float():
    f = 1.0
    for i in range(N):
        f = f * 1.0001 + 0.5
    return f


def b_fib():
    def fib(n):
        if n < 2:
            return n
        return fib(n - 1) + fib(n - 2)
    return fib(24)


bench("arith", b_arith)
bench("list", b_list)
bench("str_concat", b_str_concat)
bench("dict", b_dict)
bench("call", b_call)
bench("while", b_while)
bench("float", b_float)
bench("fib", b_fib)
