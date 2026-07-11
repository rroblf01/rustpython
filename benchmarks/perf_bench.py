#!/usr/bin/env python3
"""
RustPython Performance Benchmark Suite
Measures speed of core operations vs CPython baseline.
"""

import time
import sys

RUNS = {
    "arithmetic": 100000,
    "global_lookup": 100000,
    "list_ops": 50000,
    "dict_ops": 50000,
    "function_call": 50000,
    "string_ops": 25000,
    "while_loop": 100000,
    "attribute_lookup": 50000,
}


def bench_arithmetic():
    """Arithmetic operations."""
    n = 0
    t0 = time.perf_counter()
    for i in range(RUNS["arithmetic"]):
        n += i
        n -= i // 2
        n *= 2
        n //= 3
        n %= 1000
    t = time.perf_counter() - t0
    print("  arithmetic ({} iters): {:.4f}s  (result={})".format(RUNS["arithmetic"], t, n))
    return t


def bench_global_lookup():
    """Global variable lookup."""
    t0 = time.perf_counter()
    for i in range(RUNS["global_lookup"]):
        x = RUNS
        y = time
        z = True
    t = time.perf_counter() - t0
    print("  global_lookup ({} iters): {:.4f}s".format(RUNS["global_lookup"], t))
    return t


def bench_list_ops():
    """List creation, append, indexing."""
    t0 = time.perf_counter()
    for i in range(RUNS["list_ops"]):
        lst = [1, 2, 3]
        lst.append(i)
        x = lst[0]
        y = lst[-1]
        z = len(lst)
    t = time.perf_counter() - t0
    print("  list_ops ({} iters): {:.4f}s".format(RUNS["list_ops"], t))
    return t


def bench_dict_ops():
    """Dict creation, get, set, in check."""
    t0 = time.perf_counter()
    for i in range(RUNS["dict_ops"]):
        d = {"a": 1, "b": 2}
        d["c"] = i
        x = d.get("a")
        y = "a" in d
        z = len(d)
    t = time.perf_counter() - t0
    print("  dict_ops ({} iters): {:.4f}s".format(RUNS["dict_ops"], t))
    return t


def bench_function_call():
    """Function call overhead."""
    def f(x):
        return x + 1

    t0 = time.perf_counter()
    for i in range(RUNS["function_call"]):
        x = f(i)
    t = time.perf_counter() - t0
    print("  function_call ({} iters): {:.4f}s".format(RUNS["function_call"], t))
    return t


def bench_string_ops():
    """String operations."""
    s = "Hello, World!"
    t0 = time.perf_counter()
    for i in range(RUNS["string_ops"]):
        x = s.upper()
        y = s.lower()
        z = s.replace("o", "x")
        w = len(s)
    t = time.perf_counter() - t0
    print("  string_ops ({} iters): {:.4f}s".format(RUNS["string_ops"], t))
    return t


def bench_while_loop():
    """While loop with arithmetic."""
    i = 0
    n = 0
    t0 = time.perf_counter()
    while i < RUNS["while_loop"]:
        n += i
        i += 1
    t = time.perf_counter() - t0
    print("  while_loop ({} iters): {:.4f}s  (result={})".format(RUNS["while_loop"], t, n))
    return t


def bench_attribute_lookup():
    """Attribute lookup on builtin objects."""
    lst = [1, 2, 3, 4, 5]
    t0 = time.perf_counter()
    for i in range(RUNS["attribute_lookup"]):
        x = lst.append
        y = lst.pop
        z = lst.reverse
    t = time.perf_counter() - t0
    print("  attribute_lookup ({} iters): {:.4f}s".format(RUNS["attribute_lookup"], t))
    return t


if __name__ == "__main__":
    # Detect which impl we're running on
    impl = "RustPython"
    try:
        if not hasattr(sys, "implementation") or "rust" not in str(sys.implementation).lower():
            impl = "CPython"
    except Exception:
        impl = "CPython"
    sep = "=" * 52
    print(sep)
    print("  Performance Benchmark - " + impl)
    print(sep)
    results = {}
    total = 0.0
    # Run each benchmark sequentially
    t = bench_arithmetic()
    results["arithmetic"] = t
    total += t
    t = bench_global_lookup()
    results["global_lookup"] = t
    total += t
    t = bench_list_ops()
    results["list_ops"] = t
    total += t
    t = bench_dict_ops()
    results["dict_ops"] = t
    total += t
    t = bench_function_call()
    results["function_call"] = t
    total += t
    t = bench_string_ops()
    results["string_ops"] = t
    total += t
    t = bench_while_loop()
    results["while_loop"] = t
    total += t
    t = bench_attribute_lookup()
    results["attribute_lookup"] = t
    total += t
    print(sep)
    print("  TOTAL: {:.4f}s".format(total))
    print(sep)
