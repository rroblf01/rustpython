#!/usr/bin/env python3
"""
JIT Benchmark Suite for RustPython.
Targets each JIT-compiled opcode individually.
Run:  python3 benchmarks/jit_bench.py
Compare: python3 benchmarks/jit_bench.py  (on CPython)
"""

import time
import sys

N = 50000

def bench(name, fn):
    t0 = time.perf_counter()
    result = fn()
    t = time.perf_counter() - t0
    print(f"  {name:25s} {N:>6} iters  {t:.4f}s  result={result}")
    return t

def bench_arithmetic():
    n = 0
    for i in range(N):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    return n

def bench_negation():
    n = 0
    for i in range(N):
        n += -i
        n -= -(-i)
    return n

def bench_not():
    r = 0
    for i in range(N):
        if not (i % 3):
            r += 1
    return r

def bench_list_build():
    r = 0
    for i in range(N):
        lst = [i, i+1, i+2, i+3]
        r += lst[0] + lst[-1]
    return r

def bench_tuple_build():
    r = 0
    for i in range(N):
        tup = (i, i+1, i+2)
        r += tup[0] + tup[1]
    return r

def bench_list_append():
    lst = []
    for i in range(N):
        lst.append(i)
    return len(lst)

def bench_contains_list():
    r = 0
    lst = list(range(100))
    for i in range(N):
        if (i % 100) in lst:
            r += 1
    return r

def bench_contains_str():
    r = 0
    s = "hello world abcdefghij"
    for i in range(N):
        if "hello" in s:
            r += 1
    return r

def bench_get_iter():
    r = 0
    for i in range(N):
        for x in [1, 2, 3]:
            r += x
    return r

def bench_function_call():
    def f(x):
        return x + 1
    r = 0
    for i in range(N):
        r = f(i)
    return r

def bench_load_attr():
    class Obj:
        def __init__(self):
            self.val = 42
    obj = Obj()
    r = 0
    for i in range(N):
        r += obj.val
    return r

def bench_mixed():
    r = 0
    for i in range(N):
        # Multiple JIT opcodes combined
        x = -i if i % 2 else i
        r += x
    return r

def main():
    print(f"\nJIT Benchmark Suite  (N={N})")
    print(f"Runtime: Python {sys.version}")
    print("-" * 60)

    bench("arithmetic", bench_arithmetic)
    bench("negation", bench_negation)
    bench("not", bench_not)
    bench("list_build", bench_list_build)
    bench("tuple_build", bench_tuple_build)
    bench("list_append", bench_list_append)
    bench("contains_list", bench_contains_list)
    bench("contains_str", bench_contains_str)
    bench("get_iter", bench_get_iter)
    bench("function_call", bench_function_call)
    bench("load_attr", bench_load_attr)
    bench("mixed", bench_mixed)

    print("-" * 60)
    print("DONE\n")

if __name__ == "__main__":
    main()
